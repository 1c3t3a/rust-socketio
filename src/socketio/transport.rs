use crate::engineio::{
    packet::{Packet as EnginePacket, PacketId as EnginePacketId},
    socket::EngineSocket,
};
use crate::error::Error;
use crate::socketio::packet::{Packet as SocketPacket, PacketId as SocketPacketId};
use if_chain::if_chain;
use rand::{thread_rng, Rng};
use std::{
    fmt::Debug,
    sync::{atomic::Ordering, RwLock},
};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::{Duration, Instant},
};

use super::event::Event;

/// The type of a callback function.
pub(crate) type Callback<I> = RwLock<Box<dyn FnMut(I) + 'static + Sync + Send>>;

/// Represents an `Ack` as given back to the caller. Holds the internal `id` as
/// well as the current ack'ed state. Holds data which will be accessible as
/// soon as the ack'ed state is set to true. An `Ack` that didn't get ack'ed
/// won't contain data.
pub struct Ack {
    pub id: i32,
    timeout: Duration,
    time_started: Instant,
    callback: Callback<String>,
}

/// Handles communication in the `socket.io` protocol.
#[derive(Clone)]
pub struct TransportClient {
    engine_socket: Arc<Mutex<EngineSocket>>,
    host: Arc<String>,
    connected: Arc<AtomicBool>,
    engineio_connected: Arc<AtomicBool>,
    on: Arc<Vec<(Event, Callback<String>)>>,
    outstanding_acks: Arc<RwLock<Vec<Ack>>>,
    // Namespace, for multiplexing messages
    nsp: Arc<Option<String>>,
}

impl TransportClient {
    /// Creates an instance of `TransportClient`.
    pub fn new<T: Into<String>>(address: T, nsp: Option<String>) -> Self {
        TransportClient {
            engine_socket: Arc::new(Mutex::new(EngineSocket::new(false))),
            host: Arc::new(address.into()),
            connected: Arc::new(AtomicBool::default()),
            engineio_connected: Arc::new(AtomicBool::default()),
            on: Arc::new(Vec::new()),
            outstanding_acks: Arc::new(RwLock::new(Vec::new())),
            nsp: Arc::new(nsp),
        }
    }

    /// Registers a new event with some callback function `F`.
    pub fn on<F>(&mut self, event: Event, callback: F) -> Result<(), Error>
    where
        F: FnMut(String) + 'static + Sync + Send,
    {
        Arc::get_mut(&mut self.on)
            .unwrap()
            .push((event, RwLock::new(Box::new(callback))));
        Ok(())
    }

    /// Connects to the server. This includes a connection of the underlying
    /// engine.io client and afterwards an opening socket.io request.
    pub fn connect(&mut self) -> Result<(), Error> {
        self.setup_callbacks()?;

        self.engine_socket
            .lock()?
            .bind(self.host.as_ref().to_string())?;

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

        self.send(&open_packet)
    }

    /// Sends a `socket.io` packet to the server using the `engine.io` client.
    pub fn send(&self, packet: &SocketPacket) -> Result<(), Error> {
        if !self.engineio_connected.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }

        // the packet, encoded as an engine.io message packet
        let engine_packet = EnginePacket::new(EnginePacketId::Message, packet.encode());

        self.engine_socket.lock()?.emit(engine_packet)
    }

    /// Emits to certain event with given data. The data needs to be JSON,
    /// otherwise this returns an `InvalidJson` error.
    pub fn emit(&self, event: Event, data: &str) -> Result<(), Error> {
        if serde_json::from_str::<serde_json::Value>(data).is_err() {
            return Err(Error::InvalidJson(data.to_owned()));
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

        self.send(&socket_packet)
    }

    /// Emits and requests an `ack`. The `ack` returns a `Arc<RwLock<Ack>>` to
    /// acquire shared mutability. This `ack` will be changed as soon as the
    /// server answered with an `ack`.
    pub fn emit_with_ack<F>(
        &mut self,
        event: Event,
        data: &str,
        timeout: Duration,
        callback: F,
    ) -> Result<(), Error>
    where
        F: FnMut(String) + 'static + Send + Sync,
    {
        if serde_json::from_str::<serde_json::Value>(data).is_err() {
            return Err(Error::InvalidJson(data.into()));
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

        let ack = Ack {
            id,
            time_started: Instant::now(),
            timeout,
            callback: RwLock::new(Box::new(callback)),
        };

        // add the ack to the tuple of outstanding acks
        self.outstanding_acks.write()?.push(ack);

        self.send(&socket_packet)?;
        Ok(())
    }

    /// Handles the incoming messages and classifies what callbacks to call and how.
    /// This method is later registered as the callback for the `on_data` event of the
    /// engineio client.
    #[inline]
    fn handle_new_message(socket_bytes: Vec<u8>, clone_self: &TransportClient) {
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
                    if let Some(function) = clone_self.get_event_callback(&Event::Error) {
                        let mut lock = function.1.write().unwrap();
                        lock(
                            String::from("Received an ConnectError frame")
                                + &socket_packet.data.unwrap_or_else(|| {
                                    String::from("\"No error message provided\"")
                                }),
                        );
                        drop(lock)
                    }
                }
                SocketPacketId::Disconnect => {
                    clone_self.connected.swap(false, Ordering::Relaxed);
                }
                SocketPacketId::Event => {
                    TransportClient::handle_event(socket_packet, clone_self).unwrap();
                }
                SocketPacketId::Ack => {
                    Self::handle_ack(socket_packet, clone_self);
                }
                SocketPacketId::BinaryEvent => {
                    // call the callback
                }
                SocketPacketId::BinaryAck => {
                    // call the callback
                }
            }
        }
    }

    /// Handles the incoming acks and classifies what callbacks to call and how.
    #[inline]
    fn handle_ack(socket_packet: SocketPacket, clone_self: &TransportClient) {
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
                if ack.id == id {
                    to_be_removed.push(index);

                    if ack.time_started.elapsed() < ack.timeout {
                        if let Some(payload) = socket_packet.clone().data {
                            spawn_scoped!({
                                let mut function = ack.callback.write().unwrap();
                                function(payload);
                                drop(function);
                            });
                        }
                    }
                }
            }
            for index in to_be_removed {
                clone_self.outstanding_acks.write().unwrap().remove(index);
            }
        }
    }

    /// Sets up the callback routes on the engine.io socket, called before
    /// opening the connection.
    fn setup_callbacks(&mut self) -> Result<(), Error> {
        let clone_self = self.clone();
        let error_callback = move |msg| {
            if let Some(function) = clone_self.get_event_callback(&Event::Error) {
                let mut lock = function.1.write().unwrap();
                lock(msg);
                drop(lock)
            }
        };

        let clone_self = self.clone();
        let open_callback = move |_| {
            clone_self.engineio_connected.swap(true, Ordering::Relaxed);
            if let Some(function) = clone_self.get_event_callback(&Event::Connect) {
                let mut lock = function.1.write().unwrap();
                lock(String::from("Connection is opened"));
                drop(lock)
            }
        };

        let clone_self = self.clone();
        let close_callback = move |_| {
            clone_self.engineio_connected.swap(false, Ordering::Relaxed);
            if let Some(function) = clone_self.get_event_callback(&Event::Close) {
                let mut lock = function.1.write().unwrap();
                lock(String::from("Connection is closed"));
                drop(lock)
            }
        };

        self.engine_socket.lock()?.on_open(open_callback)?;

        self.engine_socket.lock()?.on_error(error_callback)?;

        self.engine_socket.lock()?.on_close(close_callback)?;

        let clone_self = self.clone();
        self.engine_socket
            .lock()?
            .on_data(move |data| Self::handle_new_message(data, &clone_self))
    }

    /// A method for handling the Event Socket Packets.
    // this could only be called with an event
    fn handle_event(
        socket_packet: SocketPacket,
        clone_self: &TransportClient,
    ) -> Result<(), Error> {
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
                                if let Some(function) = clone_self.get_event_callback(&Event::Custom(event)) {
                                    spawn_scoped!({
                                        let mut lock = function.1.write().unwrap();
                                        lock(contents[1].to_string());
                                        drop(lock);
                                    });
                                }
                            }
                        } else if let Some(function) = clone_self.get_event_callback(&Event::Message) {
                            spawn_scoped!({
                                let mut lock = function.1.write().unwrap();
                                lock(contents[0].to_string());
                                drop(lock);
                            });
                        }
                    }

            }
        }
        Ok(())
    }

    /// A convenient method for finding a callback for a certain event.
    #[inline]
    fn get_event_callback(&self, event: &Event) -> Option<&(Event, Callback<String>)> {
        self.on.iter().find(|item| item.0 == *event)
    }
}

impl Debug for Ack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "Ack(id: {:?}), timeout: {:?}, time_started: {:?}, callback: {}",
            self.id, self.timeout, self.time_started, "Fn(String)",
        ))
    }
}

impl Debug for TransportClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("TransportClient(engine_socket: {:?}, host: {:?}, connected: {:?}, engineio_connected: {:?}, on: <defined callbacks>, outstanding_acks: {:?}, nsp: {:?})",
            self.engine_socket,
            self.host,
            self.connected,
            self.engineio_connected,
            self.outstanding_acks,
            self.nsp,
        ))
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use super::*;
    /// The socket.io server for testing runs on port 4200
    const SERVER_URL: &str = "http://localhost:4200";

    #[test]
    fn it_works() {
        let mut socket = TransportClient::new(SERVER_URL, None);

        assert!(socket
            .on("test".into(), |s| {
                println!("{}", s);
            })
            .is_ok());

        assert!(socket.connect().is_ok());

        let ack_callback = |message: String| {
            println!("Yehaa! My ack got acked?");
            println!("Ack data: {}", message);
        };

        assert!(socket
            .emit_with_ack("test".into(), "123", Duration::from_secs(10), ack_callback)
            .is_ok());
    }
}
