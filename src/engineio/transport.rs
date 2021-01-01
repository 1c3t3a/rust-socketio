use crate::engineio::packet::{decode_payload, encode_payload, Error, Packet, PacketId};
use crypto::{digest::Digest, sha1::Sha1};

use rand::{thread_rng, Rng};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{sync::{Arc, Mutex, RwLock, atomic::AtomicBool, mpsc::{Receiver, channel}}, time::Instant};

#[derive(Debug, Clone)]
enum TransportType {
    Polling(Arc<Mutex<Client>>),
}

type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I)>>>>;

#[derive(Clone)]
pub struct TransportClient {
    transport: TransportType,
    pub on_error: Callback<String>,
    pub on_open: Callback<()>,
    pub on_close: Callback<()>,
    pub on_data: Callback<Vec<u8>>,
    pub on_packet: Callback<Packet>,
    connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    host_address: Arc<Mutex<Option<String>>>,
    connection_data: Option<HandshakeData>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct HandshakeData {
    sid: String,
    upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    ping_interval: i32,
    #[serde(rename = "pingTimeout")]
    ping_timeout: i32,
}

unsafe impl Send for TransportClient {}
unsafe impl Sync for TransportClient {}

impl TransportClient {
    pub fn new() -> Self {
        TransportClient {
            transport: TransportType::Polling(Arc::new(Mutex::new(Client::new()))),
            on_error: Arc::new(RwLock::new(None)),
            on_open: Arc::new(RwLock::new(None)),
            on_close: Arc::new(RwLock::new(None)),
            on_data: Arc::new(RwLock::new(None)),
            on_packet: Arc::new(RwLock::new(None)),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            host_address: Arc::new(Mutex::new(None)),
            connection_data: None,
        }
    }

    pub fn set_on_open<F>(&mut self, function: F)
    where
        F: Fn(()) + 'static,
    {
        let mut on_open = self.on_open.write().unwrap();
        *on_open = Some(Box::new(function));
        drop(on_open);
    }

    pub fn set_on_error<F>(&mut self, function: F)
    where
        F: Fn(String) + 'static,
    {
        let mut on_error = self.on_error.write().unwrap();
        *on_error = Some(Box::new(function));
        drop(on_error);
    }

    pub fn set_on_packet<F>(&mut self, function: F)
    where
        F: Fn(Packet) + 'static,
    {
        let mut on_packet = self.on_packet.write().unwrap();
        *on_packet = Some(Box::new(function));
        drop(on_packet);
    }

    pub fn set_on_data<F>(&mut self, function: F)
    where
        F: Fn(Vec<u8>) + 'static,
    {
        let mut on_data = self.on_data.write().unwrap();
        *on_data = Some(Box::new(function));
        drop(on_data);
    }

    pub fn set_on_close<F>(&mut self, function: F)
    where
        F: Fn(()) + 'static,
    {
        let mut on_close = self.on_close.write().unwrap();
        *on_close = Some(Box::new(function));
        drop(on_close);
    }

    pub async fn open(&mut self, address: String) -> Result<(), Error> {
        // TODO: Check if Relaxed is appropiate -> change all occurences if not
        if self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }

        match &mut self.transport {
            TransportType::Polling(client) => {
                // build the query path, random_t is used to prevent browser caching
                let query_path = &format!(
                    "/engine.io/?EIO=4&transport=polling&t={}",
                    TransportClient::get_random_t()
                )[..];

                if let Ok(full_address) = Url::parse(&(address.clone() + query_path)[..]) {
                    self.host_address = Arc::new(Mutex::new(Some(address)));

                    let client = client.lock().unwrap();
                    let response = client.get(full_address).send().await?.text().await?;
                    drop(client);

                    if let Ok(connection_data) = serde_json::from_str(&response[1..]) {
                        self.connection_data = dbg!(connection_data);

                        let function = self.on_open.read().unwrap();
                        if let Some(function) = function.as_ref() {
                            function(());
                        }
                        drop(function);

                        *Arc::get_mut(&mut self.connected).unwrap() = AtomicBool::from(true);
                        self.emit(Packet::new(PacketId::Pong, Vec::new())).await?;

                        return Ok(());
                    }
                    return Err(Error::HandshakeError(response));
                }
                return Err(Error::InvalidUrl(address));
            }
        }
    }

    pub async fn emit(&self, packet: Packet) -> Result<(), Error> {
        if !self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        match &self.transport {
            TransportType::Polling(client) => {
                let query_path = &format!(
                    "/engine.io/?EIO=4&transport=polling&t={}&sid={}",
                    TransportClient::get_random_t(),
                    self.connection_data.as_ref().unwrap().sid
                );

                let host = self.host_address.lock().unwrap().clone();
                let address =
                    Url::parse(&(host.as_ref().unwrap().to_owned() + query_path)[..]).unwrap();
                drop(host);

                let data = encode_payload(vec![packet]);
                let client = client.lock().unwrap().clone();
                let status = client
                    .post(address)
                    .body(data)
                    .send()
                    .await?
                    .status()
                    .as_u16();
                drop(client);

                if status != 200 {
                    return Err(Error::HttpError(status));
                }

                Ok(())
            }
        }
    }

    pub async fn poll_cycle(&self) -> Result<(), Error> {
        if !self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }

        while self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            match &self.transport {
                TransportType::Polling(client) => {
                    let query_path = &format!(
                        "/engine.io/?EIO=4&transport=polling&t={}&sid={}",
                        TransportClient::get_random_t(),
                        self.connection_data.as_ref().unwrap().sid
                    );

                    let host = self.host_address.lock().unwrap().clone();
                    let address =
                        Url::parse(&(host.as_ref().unwrap().to_owned() + query_path)[..]).unwrap();
                    drop(host);

                    let client = client.lock().unwrap().clone();
                    // TODO: check if to_vec is inefficient here
                    let response = client.get(address).send().await?.bytes().await?.to_vec();
                    drop(client);
                    let packets = decode_payload(response)?;

                    for packet in packets {
                        {
                            let on_packet = self.on_packet.read().unwrap();
                            // call the packet callback
                            // TODO: execute this in a new thread
                            if let Some(function) = on_packet.as_ref() {
                                function(packet.clone());
                            }
                        }
                        // check for the appropiate action or callback
                        match packet.packet_id {
                            PacketId::Message => {
                                let on_data = self.on_data.read().unwrap();
                                // TODO: execute this in a new thread
                                if let Some(function) = on_data.as_ref() {
                                    function(packet.data);
                                }
                                drop(on_data);
                            }

                            PacketId::Close => {
                                dbg!("Received close!");
                                // TODO: set close to false
                                let on_close = self.on_close.read().unwrap();
                                if let Some(function) = on_close.as_ref() {
                                    function(());
                                }
                                drop(on_close);
                                break;
                            }
                            PacketId::Open => {
                                // this will never happen as the client connects to the server and the open packet
                                // is already received in the 'open' method
                                unreachable!();
                            }
                            PacketId::Upgrade => {
                                dbg!("Received upgrade!");
                                todo!("Upgrade the connection, but only if possible");
                            }
                            PacketId::Ping => {
                                dbg!("Received ping!");
                                self.emit(Packet::new(PacketId::Pong, Vec::new())).await?;
                            }
                            PacketId::Pong => {
                                // this will never happen as the pong packet is only sent by the client
                                unreachable!();
                            }
                            PacketId::Noop => (),
                        }
                    }
                }
            }
        }
        return Ok(());
    }

    // Produces a random String that is used to prevent browser caching.
    // TODO: Check if there is a more efficient way
    fn get_random_t() -> String {
        let mut hasher = Sha1::new();
        let mut rng = thread_rng();
        let arr: [u8; 32] = rng.gen();
        hasher.input(&arr);
        hasher.result_str()
    }
}

#[cfg(test)]
mod test {
    use crate::engineio::packet::{Packet, PacketId};

    use super::*;

    #[actix_rt::test]
    async fn test_connection() {
        let mut socket = TransportClient::new();
        socket
            .open("http://localhost:4200".to_owned())
            .await
            .unwrap();

        socket
            .emit(Packet::new(
                PacketId::Message,
                "HelloWorld".to_string().into_bytes(),
            ))
            .await
            .unwrap();

        socket.on_data = Arc::new(RwLock::new(Some(Box::new(|data| {
            println!("Received: {:?}", std::str::from_utf8(&data).unwrap());
        }))));

        socket.poll_cycle().await.unwrap();
    }
}
