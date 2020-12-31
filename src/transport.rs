use crate::packet::{decode_payload, encode_payload, Error, Packet, PacketId};
use crypto::{digest::Digest, sha1::Sha1};

use rand::{thread_rng, Rng};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Instant,
};

#[derive(Debug, Clone)]
enum TransportType {
    Polling(Arc<Mutex<Client>>),
}

// do we might need a lock here? -> I would say yes, at least for message events
type Callback<I> = Arc<Mutex<Option<Box<dyn Fn(I)>>>>;

#[derive(Clone)]
struct TransportClient {
    transport: TransportType,
    on_error: Callback<String>,
    on_open: Callback<()>,
    on_close: Callback<()>,
    on_data: Callback<Vec<u8>>,
    on_packet: Callback<Packet>,
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

impl TransportClient {
    pub fn new() -> Self {
        TransportClient {
            transport: TransportType::Polling(Arc::new(Mutex::new(Client::new()))),
            on_error: Arc::new(Mutex::new(None)),
            on_open: Arc::new(Mutex::new(None)),
            on_close: Arc::new(Mutex::new(None)),
            on_data: Arc::new(Mutex::new(None)),
            on_packet: Arc::new(Mutex::new(None)),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            host_address: Arc::new(Mutex::new(None)),
            connection_data: None,
        }
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

                        let function = self.on_open.lock().unwrap();
                        if let Some(function) = function.as_ref() {
                            function(());
                        }
                        drop(function);

                        *Arc::get_mut(&mut self.connected).unwrap() = AtomicBool::from(true);
                        return Ok(());
                    }
                    return Err(Error::HandshakeError(response));
                }
                return Err(Error::InvalidUrl(address));
            }
        }
    }

    pub async fn emit(&mut self, packet: Packet) -> Result<(), Error> {
        if !self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        match &mut self.transport {
            TransportType::Polling(client) => {
                let query_path = &format!(
                    "/engine.io/?EIO=4&transport=polling&t={}&sid={}",
                    TransportClient::get_random_t(),
                    self.connection_data.as_ref().unwrap().sid
                );

                let host = self.host_address.lock().unwrap();
                let address =
                    Url::parse(&(host.as_ref().unwrap().to_owned() + query_path)[..]).unwrap();
                drop(host);

                let data = encode_payload(vec![packet]);
                let client = client.lock().unwrap();
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

    async fn poll_cycle(&mut self) -> Result<(), Error> {
        if !self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        println!("Here");

        while self.connected.load(std::sync::atomic::Ordering::Relaxed) {
            match &mut self.transport {
                TransportType::Polling(client) => {
                    let query_path = &format!(
                        "/engine.io/?EIO=4&transport=polling&t={}&sid={}",
                        TransportClient::get_random_t(),
                        self.connection_data.as_ref().unwrap().sid
                    );

                    let host = self.host_address.lock().unwrap();
                    let address =
                        Url::parse(&(host.as_ref().unwrap().to_owned() + query_path)[..]).unwrap();
                    drop(host);

                    let client = client.lock().unwrap();
                    // TODO: check if to_vec is inefficient here
                    let response = client.get(address).send().await?.bytes().await?.to_vec();
                    drop(client);
                    let packets = decode_payload(response)?;

                    for packet in packets {
                        let on_packet = self.on_packet.lock().unwrap();
                        // call the packet callback
                        // TODO: execute this in a new thread
                        if let Some(function) = on_packet.as_ref() {
                            function(packet.clone());
                        }
                        drop(on_packet);

                        // check for the appropiate action or callback
                        match packet.packet_id {
                            PacketId::Message => {
                                let on_data = self.on_data.lock().unwrap();
                                // TODO: execute this in a new thread
                                if let Some(function) = on_data.as_ref() {
                                    function(packet.data);
                                }
                                drop(on_data);
                            }

                            PacketId::Close => {
                                dbg!("Received close!");
                                *Arc::get_mut(&mut self.connected).unwrap() =
                                    AtomicBool::from(false);
                                let on_close = self.on_close.lock().unwrap();
                                if let Some(function) = on_close.as_ref() {
                                    function(());
                                }
                                drop(on_close);
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

                                let mut last_ping = self.last_ping.lock().unwrap();
                                *last_ping = Instant::now();
                                drop(last_ping);

                                self.emit(Packet::new(PacketId::Pong, Vec::new())).await?;

                                let mut last_pong = self.last_pong.lock().unwrap();
                                *last_pong = Instant::now();
                                drop(last_pong);
                            }
                            PacketId::Pong => {
                                // this will never happen as the pong packet is only sent by the client
                                unreachable!();
                            }
                            PacketId::Noop => (),
                        }
                    }

                    let last_pong = self.last_pong.lock().unwrap();
                    let is_server_inactive = last_pong.elapsed().as_millis()
                        > self.connection_data.as_ref().unwrap().ping_timeout as u128;
                    drop(last_pong);

                    if is_server_inactive {
                        // server is unresponsive, send close packet and close connection
                        self.emit(Packet::new(PacketId::Close, Vec::new())).await?;
                        *Arc::get_mut(&mut self.connected).unwrap() = AtomicBool::from(false);
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
    use crate::packet::PacketId;

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

        socket.on_data = Arc::new(Mutex::new(Some(Box::new(|data| {
            println!("Received: {:?}", std::str::from_utf8(&data).unwrap());
        }))));

        socket.poll_cycle().await.unwrap();
    }
}
