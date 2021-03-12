use crate::error::{Error, Result};
use crate::socketio::packet::{Packet as SocketPacket, PacketId as SocketPacketId};
use crate::{
    engineio::{
        packet::{Packet as EnginePacket, PacketId as EnginePacketId},
        socket::EngineSocket,
    },
    Socket,
};
use bytes::Bytes;
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

use super::{event::Event, payload::Payload};

/// The type of a callback function.
pub(crate) type Callback<I> = RwLock<Box<dyn FnMut(I, Socket) + 'static + Sync + Send>>;

pub(crate) type EventCallback = (Event, Callback<Payload>);
/// Represents an `Ack` as given back to the caller. Holds the internal `id` as
/// well as the current ack'ed state. Holds data which will be accessible as
/// soon as the ack'ed state is set to true. An `Ack` that didn't get ack'ed
/// won't contain data.
pub struct Ack {
    pub id: i32,
    timeout: Duration,
    time_started: Instant,
    callback: Callback<Payload>,
}

/// Handles communication in the `socket.io` protocol.
#[derive(Clone)]
pub struct TransportClient {
    engine_socket: Arc<Mutex<EngineSocket>>,
    host: Arc<String>,
    connected: Arc<AtomicBool>,
    engineio_connected: Arc<AtomicBool>,
    on: Arc<Vec<EventCallback>>,
    outstanding_acks: Arc<RwLock<Vec<Ack>>>,
    // used to detect unfinished binary events as, as the attachements
    // gets send in a seperate packet
    unfinished_packet: Arc<RwLock<Option<SocketPacket>>>,
    // namespace, for multiplexing messages
    pub(crate) nsp: Arc<Option<String>>,
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
            unfinished_packet: Arc::new(RwLock::new(None)),
            nsp: Arc::new(nsp),
        }
    }

    /// Registers a new event with some callback function `F`.
    pub fn on<F>(&mut self, event: Event, callback: F) -> Result<()>
    where
        F: FnMut(Payload, Socket) + 'static + Sync + Send,
    {
        Arc::get_mut(&mut self.on)
            .unwrap()
            .push((event, RwLock::new(Box::new(callback))));
        Ok(())
    }

    /// Connects to the server. This includes a connection of the underlying
    /// engine.io client and afterwards an opening socket.io request.
    pub fn connect(&mut self) -> Result<()> {
        self.setup_callbacks()?;

        self.engine_socket
            .lock()?
            .bind(self.host.as_ref().to_string())?;

        let default = String::from("/");
        // construct the opening packet
        let open_packet = SocketPacket::new(
            SocketPacketId::Connect,
            self.nsp.as_ref().as_ref().unwrap_or(&default).to_owned(),
            None,
            None,
            None,
            None,
        );

        self.send(&open_packet)
    }

    /// Sends a `socket.io` packet to the server using the `engine.io` client.
    pub fn send(&self, packet: &SocketPacket) -> Result<()> {
        if !self.engineio_connected.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }

        // the packet, encoded as an engine.io message packet
        let engine_packet = EnginePacket::new(EnginePacketId::Message, packet.encode());

        self.engine_socket.lock()?.emit(engine_packet)
    }

    /// Sends a single binary attachement to the server. This method
    /// should only be called if a `BinaryEvent` or `BinaryAck` was
    /// send to the server mentioning this attachement in it's
    /// `attachements` field.
    fn send_binary_attachement(&self, attachement: Bytes) -> Result<()> {
        if !self.engineio_connected.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }

        self.engine_socket
            .lock()?
            .emit_binary_attachement(attachement)
    }

    /// Emits to certain event with given data. The data needs to be JSON,
    /// otherwise this returns an `InvalidJson` error.
    pub fn emit(&self, event: Event, data: Payload) -> Result<()> {
        let default = String::from("/");
        let nsp = self.nsp.as_ref().as_ref().unwrap_or(&default);

        let is_string_packet = matches!(&data, &Payload::String(_));
        let socket_packet = self.build_packet_for_payload(data, event, nsp, None)?;

        if is_string_packet {
            self.send(&socket_packet)
        } else {
            // first send the raw packet announcing the attachement
            self.send(&socket_packet)?;

            // then send the attachement
            // unwrapping here is safe as this is a binary payload
            self.send_binary_attachement(socket_packet.binary_data.unwrap())
        }
    }

    /// Returns a packet for a payload, could be used for bot binary and non binary
    /// events and acks.
    #[inline]
    fn build_packet_for_payload<'a>(
        &'a self,
        payload: Payload,
        event: Event,
        nsp: &'a str,
        id: Option<i32>,
    ) -> Result<SocketPacket> {
        match payload {
            Payload::Binary(bin_data) => Ok(SocketPacket::new(
                if id.is_some() {
                    SocketPacketId::BinaryAck
                } else {
                    SocketPacketId::BinaryEvent
                },
                nsp.to_owned(),
                Some(serde_json::Value::String(event.into()).to_string()),
                Some(bin_data),
                id,
                Some(1),
            )),
            Payload::String(str_data) => {
                if serde_json::from_str::<serde_json::Value>(&str_data).is_err() {
                    return Err(Error::InvalidJson(str_data));
                }

                let payload = format!("[\"{}\",{}]", String::from(event), str_data);

                Ok(SocketPacket::new(
                    SocketPacketId::Event,
                    nsp.to_owned(),
                    Some(payload),
                    None,
                    id,
                    None,
                ))
            }
        }
    }

    /// Emits and requests an `ack`. The `ack` returns a `Arc<RwLock<Ack>>` to
    /// acquire shared mutability. This `ack` will be changed as soon as the
    /// server answered with an `ack`.
    pub fn emit_with_ack<F>(
        &mut self,
        event: Event,
        data: Payload,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: FnMut(Payload, Socket) + 'static + Send + Sync,
    {
        let id = thread_rng().gen_range(0..999);
        let default = String::from("/");
        let nsp = self.nsp.as_ref().as_ref().unwrap_or(&default);
        let socket_packet = self.build_packet_for_payload(data, event, nsp, Some(id))?;

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
    fn handle_new_message(socket_bytes: Bytes, clone_self: &TransportClient) {
        let mut is_finalized_packet = false;
        // either this is a complete packet or the rest of a binary packet (as attachements are
        // sent in a seperate packet).
        let decoded_packet = if clone_self.unfinished_packet.read().unwrap().is_some() {
            // this must be an attachement, so parse it
            let mut unfinished_packet = clone_self.unfinished_packet.write().unwrap();
            let mut finalized_packet = unfinished_packet.take().unwrap();
            finalized_packet.binary_data = Some(socket_bytes);

            is_finalized_packet = true;
            Ok(finalized_packet)
        } else {
            // this is a normal packet, so decode it
            SocketPacket::decode_bytes(&socket_bytes)
        };

        if let Ok(socket_packet) = decoded_packet {
            let default = String::from("/");
            if socket_packet.nsp != *clone_self.nsp.as_ref().as_ref().unwrap_or(&default) {
                return;
            }

            match socket_packet.packet_type {
                SocketPacketId::Connect => {
                    clone_self.connected.swap(true, Ordering::Relaxed);
                }
                SocketPacketId::ConnectError => {
                    clone_self.connected.swap(false, Ordering::Relaxed);
                    if let Some(function) = clone_self.get_event_callback(&Event::Error) {
                        spawn_scoped!({
                            let mut lock = function.1.write().unwrap();
                            let socket = Socket {
                                transport: clone_self.clone(),
                            };
                            lock(
                                Payload::String(
                                    String::from("Received an ConnectError frame")
                                        + &socket_packet.data.unwrap_or_else(|| {
                                            String::from("\"No error message provided\"")
                                        }),
                                ),
                                socket,
                            );
                            drop(lock);
                        });
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
                    // in case of a binary event, check if this is the attachement or not and
                    // then either handle the event or set the open packet
                    if is_finalized_packet {
                        Self::handle_binary_event(socket_packet, clone_self).unwrap();
                    } else {
                        *clone_self.unfinished_packet.write().unwrap() = Some(socket_packet);
                    }
                }
                SocketPacketId::BinaryAck => {
                    if is_finalized_packet {
                        Self::handle_ack(socket_packet, clone_self);
                    } else {
                        *clone_self.unfinished_packet.write().unwrap() = Some(socket_packet);
                    }
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
                        if let Some(ref payload) = socket_packet.data {
                            spawn_scoped!({
                                let mut function = ack.callback.write().unwrap();
                                let socket = Socket {
                                    transport: clone_self.clone(),
                                };
                                function(Payload::String(payload.to_owned()), socket);
                                drop(function);
                            });
                        }
                        if let Some(ref payload) = socket_packet.binary_data {
                            spawn_scoped!({
                                let mut function = ack.callback.write().unwrap();
                                let socket = Socket {
                                    transport: clone_self.clone(),
                                };
                                function(Payload::Binary(payload.to_owned()), socket);
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
    fn setup_callbacks(&mut self) -> Result<()> {
        let clone_self: TransportClient = self.clone();
        let error_callback = move |msg| {
            if let Some(function) = clone_self.get_event_callback(&Event::Error) {
                let mut lock = function.1.write().unwrap();
                let socket = Socket {
                    transport: clone_self.clone(),
                };
                lock(Payload::String(msg), socket);
                drop(lock)
            }
        };

        let clone_self = self.clone();
        let open_callback = move |_| {
            clone_self.engineio_connected.swap(true, Ordering::Relaxed);
            if let Some(function) = clone_self.get_event_callback(&Event::Connect) {
                let mut lock = function.1.write().unwrap();
                let socket = Socket {
                    transport: clone_self.clone(),
                };
                lock(
                    Payload::String(String::from("Connection is opened")),
                    socket,
                );
                drop(lock)
            }
        };

        let clone_self = self.clone();
        let close_callback = move |_| {
            clone_self.engineio_connected.swap(false, Ordering::Relaxed);
            if let Some(function) = clone_self.get_event_callback(&Event::Close) {
                let mut lock = function.1.write().unwrap();
                let socket = Socket {
                    transport: clone_self.clone(),
                };
                lock(
                    Payload::String(String::from("Connection is closed")),
                    socket,
                );
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

    /// Handles a binary event.
    #[inline]
    fn handle_binary_event(
        socket_packet: SocketPacket,
        clone_self: &TransportClient,
    ) -> Result<()> {
        let event = if let Some(string_data) = socket_packet.data {
            string_data.replace("\"", "").into()
        } else {
            Event::Message
        };

        if let Some(binary_payload) = socket_packet.binary_data {
            if let Some(function) = clone_self.get_event_callback(&event) {
                spawn_scoped!({
                    let mut lock = function.1.write().unwrap();
                    let socket = Socket {
                        transport: clone_self.clone(),
                    };
                    lock(Payload::Binary(binary_payload), socket);
                    drop(lock);
                });
            }
        }
        Ok(())
    }

    /// A method for handling the Event Socket Packets.
    // this could only be called with an event
    fn handle_event(socket_packet: SocketPacket, clone_self: &TransportClient) -> Result<()> {
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
                            if let serde_json::Value::String(event) = &contents[0] {
                                if let Some(function) = clone_self.get_event_callback(&Event::Custom(event.to_owned())) {
                                    spawn_scoped!({
                                        let mut lock = function.1.write().unwrap();
                                        let socket = Socket {
                                            transport: clone_self.clone(),
                                        };
                                        lock(Payload::String(contents[1].to_string()), socket);
                                        drop(lock);
                                    });
                                }
                            }
                        } else if let Some(function) = clone_self.get_event_callback(&Event::Message) {
                            spawn_scoped!({
                                let mut lock = function.1.write().unwrap();
                                let socket = Socket {
                                    transport: clone_self.clone(),
                                };
                                lock(Payload::String(contents[0].to_string()), socket);
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
    fn get_event_callback(&self, event: &Event) -> Option<&(Event, Callback<Payload>)> {
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
            .on("test".into(), |message, _| {
                if let Payload::String(st) = message {
                    println!("{}", st)
                }
            })
            .is_ok());

        assert!(socket.connect().is_ok());

        let ack_callback = |message: Payload, _| {
            println!("Yehaa! My ack got acked?");
            match message {
                Payload::String(str) => {
                    println!("Received string ack");
                    println!("Ack data: {}", str);
                }
                Payload::Binary(bin) => {
                    println!("Received binary ack");
                    println!("Ack data: {:#?}", bin);
                }
            }
        };

        assert!(socket
            .emit_with_ack(
                "test".into(),
                Payload::String("123".to_owned()),
                Duration::from_secs(10),
                ack_callback
            )
            .is_ok());
    }
}
