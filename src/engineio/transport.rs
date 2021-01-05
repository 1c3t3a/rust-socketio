use crate::engineio::packet::{decode_payload, encode_payload, Packet, PacketId};
use crate::error::Error;
use adler32::adler32;
use std::time::SystemTime;
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

/// The different types of transport. Used for actually
/// transmitting the payload.
#[derive(Debug, Clone)]
enum TransportType {
    Polling(Arc<Mutex<Client>>),
}

/// Type of a callback function. (Normal closures can be passed in here).
type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I) + 'static + Sync + Send>>>>;

/// A client that handles the plain transmission of packets in the engine.io protocol.
/// Used by the wrapper EngineSocket. This struct also holds the callback functions.
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
    connection_data: Arc<Option<HandshakeData>>,
    engine_io_mode: Arc<AtomicBool>,
}

/// The data that get's exchanged in the handshake. It's content
/// is usually defined by the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct HandshakeData {
    sid: String,
    upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    ping_interval: u64,
    #[serde(rename = "pingTimeout")]
    ping_timeout: u64,
}

// TODO: make this safe.
unsafe impl Send for TransportClient {}
unsafe impl Sync for TransportClient {}

impl TransportClient {
    /// Creates an instance.
    pub fn new(engine_io_mode: bool) -> Self {
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
            connection_data: Arc::new(None),
            engine_io_mode: Arc::new(AtomicBool::from(engine_io_mode)),
        }
    }

    /// Registers an on_open callback.
    pub fn set_on_open<F>(&mut self, function: F)
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        let mut on_open = self.on_open.write().unwrap();
        *on_open = Some(Box::new(function));
        drop(on_open);
    }

    /// Registers an on_error callback.
    pub fn set_on_error<F>(&mut self, function: F)
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        let mut on_error = self.on_error.write().unwrap();
        *on_error = Some(Box::new(function));
        drop(on_error);
    }

    /// Registers an on_packet callback.
    pub fn set_on_packet<F>(&mut self, function: F)
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        let mut on_packet = self.on_packet.write().unwrap();
        *on_packet = Some(Box::new(function));
        drop(on_packet);
    }

    /// Registers an on_data callback.
    pub fn set_on_data<F>(&mut self, function: F)
    where
        F: Fn(Vec<u8>) + 'static + Sync + Send,
    {
        let mut on_data = self.on_data.write().unwrap();
        *on_data = Some(Box::new(function));
        drop(on_data);
    }

    /// Registers an on_close callback.
    pub fn set_on_close<F>(&mut self, function: F)
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        let mut on_close = self.on_close.write().unwrap();
        *on_close = Some(Box::new(function));
        drop(on_close);
    }

    /// Opens the connection to a certain server. This includes an opening GET request to the server.
    /// The server passes back the handshake data in the response. Afterwards a first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub async fn open(&mut self, address: String) -> Result<(), Error> {
        // TODO: Check if Relaxed is appropiate -> change all occurences if not
        if self.connected.load(Ordering::Relaxed) {
            return Ok(());
        }

        match &mut self.transport {
            TransportType::Polling(client) => {
                // build the query path, random_t is used to prevent browser caching
                let query_path = &format!(
                    "/{}/?EIO=4&transport=polling&t={}",
                    if self.engine_io_mode.load(Ordering::Relaxed) {
                        "engine.io"
                    } else {
                        "socket.io"
                    },
                    TransportClient::get_random_t(),
                )[..];

                if let Ok(full_address) = Url::parse(&(address.clone() + query_path)[..]) {
                    self.host_address = Arc::new(Mutex::new(Some(address)));

                    let response = client
                        .lock()
                        .unwrap()
                        .get(full_address)
                        .send()
                        .await?
                        .text()
                        .await?;

                    // the response contains the handshake data
                    if let Ok(conn_data) = serde_json::from_str(&response[1..]) {
                        self.connection_data = Arc::new(conn_data);

                        // call the on_open callback
                        let function = self.on_open.read().unwrap();
                        if let Some(function) = function.as_ref() {
                            spawn_scoped!(function(()));
                        }
                        drop(function);

                        // set the last ping to now and set the connected state
                        *self.last_ping.lock().unwrap() = Instant::now();
                        *Arc::get_mut(&mut self.connected).unwrap() = AtomicBool::from(true);

                        // emit a pong packet to keep trigger the ping cycle on the server
                        self.emit(Packet::new(PacketId::Pong, Vec::new())).await?;

                        return Ok(());
                    }

                    let error = Error::HandshakeError(response);
                    self.call_error_callback(format!("{}", error));
                    return Err(error);
                }

                let error = Error::InvalidUrl(address);
                self.call_error_callback(format!("{}", error));
                return Err(error);
            }
        }
    }

    /// Sends a packet to the server
    pub async fn emit(&self, packet: Packet) -> Result<(), Error> {
        if !self.connected.load(Ordering::Relaxed) {
            let error = Error::ActionBeforeOpen;
            self.call_error_callback(format!("{}", error));
            return Err(error);
        }

        match &self.transport {
            TransportType::Polling(client) => {
                let query_path = &format!(
                    "/{}/?EIO=4&transport=polling&t={}&sid={}",
                    if self.engine_io_mode.load(Ordering::Relaxed) {
                        "engine.io"
                    } else {
                        "socket.io"
                    },
                    TransportClient::get_random_t(),
                    Arc::as_ref(&self.connection_data).as_ref().unwrap().sid
                );

                // build the target address
                let host = self.host_address.lock().unwrap().clone();
                let address =
                    Url::parse(&(host.as_ref().unwrap().to_owned() + query_path)[..]).unwrap();
                drop(host);

                // send a post request with the encoded payload as body
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
                    let error = Error::HttpError(status);
                    self.call_error_callback(format!("{}", error));
                    return Err(error);
                }

                Ok(())
            }
        }
    }

    /// Performs the server long polling procedure as long as the client is connected.
    /// This should run seperately at all time to ensure proper response handling from the server.
    pub async fn poll_cycle(&self) -> Result<(), Error> {
        if !self.connected.load(Ordering::Relaxed) {
            let error = Error::ActionBeforeOpen;
            self.call_error_callback(format!("{}", error));
            return Err(error);
        }
        let client = Client::new();

        // as we don't have a mut self, the last_ping needs to be safed for later
        let mut last_ping = self.last_ping.clone().lock().unwrap().clone();
        // the time after we assume the server to be timed out
        let server_timeout = Duration::from_millis(
            Arc::as_ref(&self.connection_data)
                .as_ref()
                .unwrap()
                .ping_timeout
                + Arc::as_ref(&self.connection_data)
                    .as_ref()
                    .unwrap()
                    .ping_interval,
        );

        while self.connected.load(Ordering::Relaxed) {
            match &self.transport {
                // we wont't use the shared client as this blocks the ressource
                // in the long polling requests
                TransportType::Polling(_) => {
                    let query_path = &format!(
                        "/{}/?EIO=4&transport=polling&t={}&sid={}",
                        if self.engine_io_mode.load(Ordering::Relaxed) {
                            "engine.io"
                        } else {
                            "socket.io"
                        },
                        TransportClient::get_random_t(),
                        Arc::as_ref(&self.connection_data).as_ref().unwrap().sid
                    );

                    let host = self.host_address.lock().unwrap().clone();
                    let address =
                        Url::parse(&(host.as_ref().unwrap().to_owned() + query_path)[..]).unwrap();
                    drop(host);

                    // TODO: check if to_vec is inefficient here
                    let response = client.get(address).send().await?.bytes().await?.to_vec();
                    let packets = decode_payload(response)?;

                    for packet in packets {
                        {
                            let on_packet = self.on_packet.read().unwrap();
                            // call the packet callback
                            if let Some(function) = on_packet.as_ref() {
                                spawn_scoped!(function(packet.clone()));
                            }
                        }
                        // check for the appropiate action or callback
                        match packet.packet_id {
                            PacketId::Message => {
                                let on_data = self.on_data.read().unwrap();
                                if let Some(function) = on_data.as_ref() {
                                    spawn_scoped!(function(packet.data));
                                }
                                drop(on_data);
                            }

                            PacketId::Close => {
                                let on_close = self.on_close.read().unwrap();
                                if let Some(function) = on_close.as_ref() {
                                    spawn_scoped!(function(()));
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
                                todo!("Upgrade the connection, but only if possible");
                            }
                            PacketId::Ping => {
                                last_ping = Instant::now();
                                self.emit(Packet::new(PacketId::Pong, Vec::new())).await?;
                            }
                            PacketId::Pong => {
                                // this will never happen as the pong packet is only sent by the client
                                unreachable!();
                            }
                            PacketId::Noop => (),
                        }
                    }

                    if server_timeout < last_ping.elapsed() {
                        // the server is unreachable
                        // TODO: Inform the others about the stop (maybe through a channel)
                        break;
                    }
                }
            }
        }
        return Ok(());
    }

    /// Produces a random String that is used to prevent browser caching.
    fn get_random_t() -> String {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        hash.to_string()
    }

    /// Calls the error callback with a given message.
    fn call_error_callback(&self, text: String) {
        let function = self.on_error.read().unwrap();
        if let Some(function) = function.as_ref() {
            spawn_scoped!(function(text));
        }
        drop(function);
    }
}

#[cfg(test)]
mod test {
    use crate::engineio::packet::{Packet, PacketId};

    use super::*;

    #[actix_rt::test]
    async fn test_connection() {
        let mut socket = TransportClient::new(true);
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
