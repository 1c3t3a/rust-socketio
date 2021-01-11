use crate::engineio::{
    packet::{Packet as EnginePacket, PacketId as EnginePacketId},
    socket::EngineSocket,
};
use crate::error::Error;
use crate::socketio::packet::{Packet as SocketPacket, PacketId as SocketPacketId};
use if_chain::if_chain;
use rand::{thread_rng, Rng};
use std::sync::{atomic::Ordering, RwLock};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};

use super::{ack::Ack, event::Event};

/// The type of a callback function.
type Callback<I> = RwLock<Box<dyn Fn(I) + 'static + Sync + Send>>;

/// A type that contains the Ack that was given to the caller as well as the time
/// the initial request was send to the server and the duration the user specifier
/// timeout.
type AckInformation = (Arc<RwLock<Ack>>, Instant, Duration);

/// A struct that handles communication in the socket.io protocol.
#[derive(Clone)]
pub struct TransportClient {
    engine_socket: Arc<Mutex<EngineSocket>>,
    host: Arc<String>,
    connected: Arc<AtomicBool>,
    engineio_connected: Arc<AtomicBool>,
    on: Arc<Vec<(Event, Callback<String>)>>,
    outstanding_acks: Arc<RwLock<Vec<AckInformation>>>,
    nsp: Arc<Option<String>>,
}

impl TransportClient {
    /// Creates an instance.
    pub fn new(address: String, nsp: Option<String>) -> Self {
        TransportClient {
            engine_socket: Arc::new(Mutex::new(EngineSocket::new(false))),
            host: Arc::new(address),
            connected: Arc::new(AtomicBool::default()),
            engineio_connected: Arc::new(AtomicBool::default()),
            on: Arc::new(Vec::new()),
            outstanding_acks: Arc::new(RwLock::new(Vec::new())),
            nsp: Arc::new(nsp),
        }
    }

    /// Registers a new event with a certain callback function F.
    pub fn on<F>(&mut self, event: Event, callback: F) -> Result<(), Error>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        Arc::get_mut(&mut self.on)
            .unwrap()
            .push((event, RwLock::new(Box::new(callback))));
        Ok(())
    }

    /// Connects to the server. This includes a connection of the underlying engine.io client and
    /// afterwards an opening socket.io request.
    pub async fn connect(&mut self) -> Result<(), Error> {
        self.setup_callbacks()?;

        self.engine_socket
            .lock()
            .unwrap()
            .bind(self.host.as_ref().to_string())
            .await?;

        // construct the opening packet
        let open_packet = SocketPacket::new(
            SocketPacketId::Connect,
            self.nsp
                .as_ref()
                .clone()
                .unwrap_or_else(|| String::from("/")),
            None,
            None,
            None,
            None,
        );

        self.send(open_packet).await
    }

    /// Sends a socket.io packet to the server using the engine.io client.
    pub async fn send(&self, packet: SocketPacket) -> Result<(), Error> {
        if !self.engineio_connected.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }

        // the packet, encoded as an engine.io message packet
        let engine_packet = EnginePacket::new(EnginePacketId::Message, packet.encode());

        self.engine_socket.lock().unwrap().emit(engine_packet).await
    }

    /// Emits to certain event with given data. The data needs to be JSON, otherwise this returns
    /// an `InvalidJson` error.
    pub async fn emit(&self, event: Event, data: String) -> Result<(), Error> {
        if serde_json::from_str::<serde_json::Value>(&data).is_err() {
            return Err(Error::InvalidJson(data));
        }

        let payload = format!("[\"{}\",{}]", String::from(event), data);

        let socket_packet = SocketPacket::new(
            SocketPacketId::Event,
            self.nsp
                .as_ref()
                .clone()
                .unwrap_or_else(|| String::from("/")),
            Some(payload),
            None,
            None,
            None,
        );

        self.send(socket_packet).await
    }

    /// Emits and requests an ack. The ack is returned a Arc<RwLock<Ack>> to acquire shared
    /// mutability. This ack will be changed as soon as the server answered with an ack.
    pub async fn emit_with_ack(
        &mut self,
        event: Event,
        data: String,
        timespan: Duration,
    ) -> Result<Arc<RwLock<Ack>>, Error> {
        if serde_json::from_str::<serde_json::Value>(&data).is_err() {
            return Err(Error::InvalidJson(data));
        }

        let payload = format!("[\"{}\",{}]", String::from(event), data);
        let id = thread_rng().gen_range(0..999);

        let socket_packet = SocketPacket::new(
            SocketPacketId::Event,
            self.nsp
                .as_ref()
                .clone()
                .unwrap_or_else(|| String::from("/")),
            Some(payload),
            None,
            Some(id),
            None,
        );

        let ack = Arc::new(RwLock::new(Ack {
            id,
            acked: false,
            data: None,
        }));
        // add the ack to the tuple of outstanding acks
        self.outstanding_acks
            .write()
            .unwrap()
            .push((ack.clone(), Instant::now(), timespan));

        self.send(socket_packet).await?;
        Ok(ack)
    }

    /// Sets up the callback routes on the engineio socket, called before opening the connection.
    fn setup_callbacks(&mut self) -> Result<(), Error> {
        let clone_self = self.clone();
        let on_data_callback = move |socket_bytes: Vec<u8>| {
            if let Ok(socket_packet) =
                SocketPacket::decode_string(std::str::from_utf8(&socket_bytes).unwrap().to_owned())
            {
                if socket_packet.nsp
                    != clone_self
                        .nsp
                        .as_ref()
                        .clone()
                        .unwrap_or_else(|| String::from("/"))
                {
                    return;
                }
                match socket_packet.packet_type {
                    SocketPacketId::Connect => {
                        clone_self.connected.swap(true, Ordering::Relaxed);
                    }
                    SocketPacketId::ConnectError => {
                        clone_self.connected.swap(false, Ordering::Relaxed);
                        if let Some(function) = clone_self.get_event_callback(Event::Error) {
                            let lock = function.1.read().unwrap();
                            lock(
                                String::from("Received an ConnectError frame")
                                    + &socket_packet.data.unwrap_or_else(|| {
                                        String::from("\"No error message provided\"")
                                    })[..],
                            );
                            drop(lock)
                        }
                    }
                    SocketPacketId::Disconnect => {
                        clone_self.connected.swap(false, Ordering::Relaxed);
                    }
                    SocketPacketId::Event => {
                        TransportClient::handle_event(socket_packet, &clone_self);
                    }
                    SocketPacketId::Ack => {
                        let mut to_be_removed = Vec::new();
                        if let Some(id) = socket_packet.id {
                            for (index, ack) in clone_self
                                .clone()
                                .outstanding_acks
                                .read()
                                .unwrap()
                                .iter()
                                .enumerate()
                            {
                                if ack.0.read().unwrap().id == id {
                                    to_be_removed.push(index);
                                    ack.0.write().unwrap().acked = ack.1.elapsed() < ack.2;
                                    if ack.0.read().unwrap().acked {
                                        if let Some(payload) = socket_packet.clone().data {
                                            ack.0.write().unwrap().data = Some(payload);
                                        }
                                    }
                                }
                            }
                            for index in to_be_removed {
                                clone_self.outstanding_acks.write().unwrap().remove(index);
                            }
                        }
                    }
                    SocketPacketId::BinaryEvent => {
                        // call the callback
                    }
                    SocketPacketId::BinaryAck => {
                        // call the callback
                    }
                }
            }
        };

        let clone_self = self.clone();
        let error_callback = move |msg| {
            if let Some(function) = clone_self.get_event_callback(Event::Error) {
                let lock = function.1.read().unwrap();
                lock(msg);
                drop(lock)
            }
        };

        let clone_self = self.clone();
        let open_callback = move |_| {
            clone_self.engineio_connected.swap(true, Ordering::Relaxed);
            if let Some(function) = clone_self.get_event_callback(Event::Connect) {
                let lock = function.1.read().unwrap();
                lock(String::from("Connection is opened"));
                drop(lock)
            }
        };

        let clone_self = self.clone();
        let close_callback = move |_| {
            clone_self.engineio_connected.swap(false, Ordering::Relaxed);
            if let Some(function) = clone_self.get_event_callback(Event::Close) {
                let lock = function.1.read().unwrap();
                lock(String::from("Connection is closed"));
                drop(lock)
            }
        };

        self.engine_socket.lock().unwrap().on_open(open_callback)?;

        self.engine_socket
            .lock()
            .unwrap()
            .on_error(error_callback)?;

        self.engine_socket
            .lock()
            .unwrap()
            .on_close(close_callback)?;

        self.engine_socket.lock().unwrap().on_data(on_data_callback)
    }

    /// A method for handling the Event Socket Packets.
    fn handle_event(socket_packet: SocketPacket, clone_self: &TransportClient) {
        // this could only be called with an event
        assert_eq!(socket_packet.packet_type, SocketPacketId::Event);

        // unwrap the potential data
        if let Some(data) = socket_packet.data {
            if_chain! {
                    // the string must be a valid json array with the event at index 0 and the
                    // payload at index 1. if no event is specified, the message callback is used
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&data);
                    if let serde_json::Value::Array(contents) = value;
                    then {
                        // check which callback to use and call it with the data if it's present
                        if data.len() > 1 {
                            if let serde_json::Value::String(event) = contents[0].clone() {
                                if let Some(function) = clone_self.get_event_callback(Event::Custom(event)) {
                                    let lock = function.1.read().unwrap();
                                    spawn_scoped!(lock(contents[1].to_string()));
                                    drop(lock)
                                }
                            }
                        } else if let Some(function) = clone_self.get_event_callback(Event::Message) {
                            let lock = function.1.read().unwrap();
                            spawn_scoped!(lock(contents[0].to_string()));
                            drop(lock);
                        }
                    }

            }
        }
    }

    /// A convenient method for finding a callback for a certain event.
    fn get_event_callback(&self, event: Event) -> Option<&(Event, Callback<String>)> {
        self.on.iter().find(|item| item.0 == event)
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;
    use tokio::time::sleep;

    use super::*;

    #[actix_rt::test]
    async fn it_works() {
        let mut socket = TransportClient::new(String::from("http://localhost:4200"), None);

        socket
            .on("test".into(), |s| {
                println!("{}", s);
            })
            .unwrap();

        socket.connect().await.unwrap();

        let id = socket
            .emit_with_ack("test".into(), "123".to_owned(), Duration::from_secs(10))
            .await
            .unwrap();

        sleep(Duration::from_secs(2)).await;

        println!("Ack acked? {}", id.read().unwrap().acked);
        println!("Ack data: {}", id.read().unwrap().data.as_ref().unwrap());

        socket
            .emit("test".into(), String::from("\"Hallo\""))
            .await
            .unwrap();
    }
}
