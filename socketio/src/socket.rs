use super::{event::Event, payload::Payload};
use crate::client::Socket as SocketIoSocket;
use crate::error::{Error, Result};
use crate::packet::{Packet as SocketPacket, PacketId as SocketPacketId};
use bytes::Bytes;
use rand::{thread_rng, Rng};
use rust_engineio::model::Socket as _;
use rust_engineio::{Packet as EnginePacket, PacketId as EnginePacketId};
use std::convert::TryFrom;
use std::thread;
use std::{
    fmt::Debug,
    sync::{atomic::Ordering, RwLock},
};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::{Duration, Instant},
};
use url::Url;

/// The type of a callback function.
// TODO: refactor SocketIoSocket out
pub(crate) type Callback<I> = RwLock<Box<dyn FnMut(I, SocketIoSocket) + 'static + Sync + Send>>;

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
pub struct Socket<EngineSocket: rust_engineio::model::Socket + 'static> {
    engine_socket: Arc<EngineSocket>,
    host: Arc<Url>,
    connected: Arc<AtomicBool>,
    // TODO: Move this to client/socket.rs
    on: Arc<Vec<EventCallback>>,
    outstanding_acks: Arc<RwLock<Vec<Ack>>>,
    // namespace, for multiplexing messages
    pub(crate) nsp: Arc<Option<String>>,
}

// TODO: change this to after .clone is refactored
// impl<EngineSocket: rust_engineio::model::Socket + 'static> Socket<EngineSocket> {
impl Socket<rust_engineio::client::Socket> {
    /// Creates an instance of `Socket`.
    pub(super) fn new<T: Into<String>>(
        address: T,
        nsp: Option<String>,
        engine_socket: rust_engineio::client::Socket,
        on_event: Option<Vec<EventCallback>>,
    ) -> Result<Self> {
        let mut url: Url = Url::parse(&address.into())?;

        if url.path() == "/" {
            url.set_path("/socket.io/");
        }

        let mut on = Vec::new();

        if let Some(on_event) = on_event {
            on = on_event;
        }

        Ok(Socket {
            engine_socket: Arc::new(engine_socket),
            host: Arc::new(url),
            connected: Arc::new(AtomicBool::default()),
            on: Arc::new(on),
            outstanding_acks: Arc::new(RwLock::new(Vec::new())),
            nsp: Arc::new(nsp),
        })
    }

    /// Connects to the server. This includes a connection of the underlying
    /// engine.io client and afterwards an opening socket.io request.
    pub(super) fn connect(&self) -> Result<()> {
        self.connect_with_thread(true)
    }

    /// Connect with optional thread to forward events to callback
    pub(super) fn connect_with_thread(&self, thread: bool) -> Result<()> {
        self.engine_socket.connect()?;

        // TODO: refactor me
        // TODO: This is needed (somewhere) to get callbacks to work.
        if thread {
            let clone_self = self.clone();
            thread::spawn(move || {
                // tries to restart a poll cycle whenever a 'normal' error occurs,
                // it just panics on network errors, in case the poll cycle returned
                // `Result::Ok`, the server receives a close frame so it's safe to
                // terminate
                let iter = clone_self.iter();
                for packet in iter {
                    if let e @ Err(Error::IncompleteResponseFromEngineIo(_)) = packet {
                        panic!("{}", e.unwrap_err())
                    }
                }
            });
        }

        // construct the opening packet
        let open_packet = SocketPacket::new(
            SocketPacketId::Connect,
            self.nsp
                .as_ref()
                .as_ref()
                .unwrap_or(&String::from("/"))
                .to_owned(),
            None,
            None,
            0,
            None,
        );

        // store the connected value as true, if the connection process fails
        // later, the value will be updated
        self.connected.store(true, Ordering::Release);
        self.send(open_packet)
    }

    /// Disconnects from the server by sending a socket.io `Disconnect` packet. This results
    /// in the underlying engine.io transport to get closed as well.
    pub(super) fn disconnect(&mut self) -> Result<()> {
        if !self.is_engineio_connected()? || !self.connected.load(Ordering::Acquire) {
            // If we are already disconnected no need to do anything
            return Ok(());
        }

        let disconnect_packet = SocketPacket::new(
            SocketPacketId::Disconnect,
            self.nsp
                .as_ref()
                .as_ref()
                .unwrap_or(&String::from("/"))
                .to_owned(),
            None,
            None,
            0,
            None,
        );

        self.send(disconnect_packet)?;
        self.connected.store(false, Ordering::Release);
        Ok(())
    }

    /// Sends a `socket.io` packet to the server using the `engine.io` client.
    pub(super) fn send(&self, packet: SocketPacket) -> Result<()> {
        if !self.is_engineio_connected()? || !self.connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionBeforeOpen());
        }

        // the packet, encoded as an engine.io message packet
        let engine_packet = EnginePacket::new(EnginePacketId::Message, Bytes::from(&packet));
        self.engine_socket.emit(engine_packet)?;

        if let Some(attachments) = packet.attachments {
            for attachment in attachments {
                let engine_packet = EnginePacket::new(EnginePacketId::MessageBinary, attachment);
                self.engine_socket.emit(engine_packet)?;
            }
        }

        Ok(())
    }

    /// Emits to certain event with given data. The data needs to be JSON,
    /// otherwise this returns an `InvalidJson` error.
    pub(super) fn emit(&self, event: Event, data: Payload) -> Result<()> {
        let default = String::from("/");
        let nsp = self.nsp.as_ref().as_ref().unwrap_or(&default);

        let socket_packet = self.build_packet_for_payload(data, event, nsp, None)?;

        self.send(socket_packet)
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
                id,
                1,
                Some(vec![bin_data]),
            )),
            Payload::String(str_data) => {
                serde_json::from_str::<serde_json::Value>(&str_data)?;

                let payload = format!("[\"{}\",{}]", String::from(event), str_data);

                Ok(SocketPacket::new(
                    SocketPacketId::Event,
                    nsp.to_owned(),
                    Some(payload),
                    id,
                    0,
                    None,
                ))
            }
        }
    }

    /// Emits and requests an `ack`. The `ack` returns a `Arc<RwLock<Ack>>` to
    /// acquire shared mutability. This `ack` will be changed as soon as the
    /// server answered with an `ack`.
    pub(super) fn emit_with_ack<F>(
        &self,
        event: Event,
        data: Payload,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: FnMut(Payload, SocketIoSocket) + 'static + Send + Sync,
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

        self.send(socket_packet)?;
        Ok(())
    }

    pub(super) fn iter(&self) -> Iter<rust_engineio::client::Socket> {
        Iter {
            socket: self,
            engine_iter: self.engine_socket.iter(),
        }
    }

    /// Handles the incoming messages and classifies what callbacks to call and how.
    /// This method is later registered as the callback for the `on_data` event of the
    /// engineio client.
    #[inline]
    fn handle_new_socketio_packet(&self, socket_packet: SocketPacket) -> Option<SocketPacket> {
        let output = socket_packet.clone();

        let default = String::from("/");
        //TODO: should nsp logic be here?
        if socket_packet.nsp != *self.nsp.as_ref().as_ref().unwrap_or(&default) {
            return None;
        }

        match socket_packet.packet_type {
            SocketPacketId::Connect => {
                self.connected.store(true, Ordering::Release);
            }
            SocketPacketId::ConnectError => {
                self.connected.store(false, Ordering::Release);
                if let Some(function) = self.get_event_callback(&Event::Error) {
                    spawn_scoped!({
                        let mut lock = function.1.write().unwrap();
                        // TODO: refactor
                        lock(
                            Payload::String(
                                String::from("Received an ConnectError frame")
                                    + &socket_packet.data.unwrap_or_else(|| {
                                        String::from("\"No error message provided\"")
                                    }),
                            ),
                            SocketIoSocket {
                                socket: self.clone(),
                            },
                        );
                        drop(lock);
                    });
                }
            }
            SocketPacketId::Disconnect => {
                self.connected.store(false, Ordering::Release);
            }
            SocketPacketId::Event => {
                self.handle_event(socket_packet);
            }
            SocketPacketId::Ack | SocketPacketId::BinaryAck => {
                self.handle_ack(socket_packet);
            }
            SocketPacketId::BinaryEvent => {
                // in case of a binary event, check if this is the attachement or not and
                // then either handle the event or set the open packet
                self.handle_binary_event(socket_packet);
            }
        }
        Some(output)
    }

    /// Handles new incoming engineio packets
    fn handle_new_engineio_packet(
        &self,
        engine_iter: &mut rust_engineio::client::Iter,
        packet: EnginePacket,
    ) -> Option<Result<SocketPacket>> {
        let socket_packet = SocketPacket::try_from(&packet.data);
        if let Err(err) = socket_packet {
            return Some(Err(err));
        }
        // SAFETY: checked above to see if it was Err
        let mut socket_packet = socket_packet.unwrap();
        // Only handle attachments if there are any
        if socket_packet.attachment_count > 0 {
            let mut attachments_left = socket_packet.attachment_count;
            let mut attachments = Vec::new();
            while attachments_left > 0 {
                let next = engine_iter.next()?;
                match next {
                    Err(err) => return Some(Err(err.into())),
                    Ok(packet) => match packet.packet_id {
                        EnginePacketId::MessageBinary | EnginePacketId::Message => {
                            attachments.push(packet.data);
                            attachments_left = attachments_left - 1;
                        }
                        _ => {
                            return Some(Err(Error::InvalidAttachmentPacketType(
                                packet.packet_id.into(),
                            )));
                        }
                    },
                }
            }
            socket_packet.attachments = Some(attachments);
        }

        let packet = self.handle_new_socketio_packet(socket_packet);
        if let Some(packet) = packet {
            return Some(Ok(packet));
        } else {
            return None;
        }
    }

    /// Handles the incoming acks and classifies what callbacks to call and how.
    #[inline]
    fn handle_ack(&self, socket_packet: SocketPacket) {
        let mut to_be_removed = Vec::new();
        if let Some(id) = socket_packet.id {
            for (index, ack) in self
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
                                // TODO: refactor
                                function(
                                    Payload::String(payload.to_owned()),
                                    SocketIoSocket {
                                        socket: self.clone(),
                                    },
                                );
                                drop(function);
                            });
                        }
                        if let Some(ref attachments) = socket_packet.attachments {
                            if let Some(payload) = attachments.get(0) {
                                spawn_scoped!({
                                    let mut function = ack.callback.write().unwrap();
                                    // TODO: refactor
                                    function(
                                        Payload::Binary(payload.to_owned()),
                                        SocketIoSocket {
                                            socket: self.clone(),
                                        },
                                    );
                                    drop(function);
                                });
                            }
                        }
                    }
                }
            }
            for index in to_be_removed {
                self.outstanding_acks.write().unwrap().remove(index);
            }
        }
    }

    /// Handles a binary event.
    #[inline]
    fn handle_binary_event(&self, socket_packet: SocketPacket) {
        let event = if let Some(string_data) = socket_packet.data {
            string_data.replace("\"", "").into()
        } else {
            Event::Message
        };

        if let Some(attachments) = socket_packet.attachments {
            if let Some(binary_payload) = attachments.get(0) {
                if let Some(function) = self.get_event_callback(&event) {
                    spawn_scoped!({
                        let mut lock = function.1.write().unwrap();
                        // TODO: refactor
                        lock(
                            Payload::Binary(binary_payload.to_owned()),
                            SocketIoSocket {
                                socket: self.clone(),
                            },
                        );
                        drop(lock);
                    });
                }
            }
        }
    }

    /// A method for handling the Event Socket Packets.
    // this could only be called with an event
    fn handle_event(&self, socket_packet: SocketPacket) {
        // unwrap the potential data
        if let Some(data) = socket_packet.data {
            // the string must be a valid json array with the event at index 0 and the
            // payload at index 1. if no event is specified, the message callback is used
            if let Ok(serde_json::Value::Array(contents)) =
                serde_json::from_str::<serde_json::Value>(&data)
            {
                let event: Event = if contents.len() > 1 {
                    contents
                        .get(0)
                        .map(|value| match value {
                            serde_json::Value::String(ev) => ev,
                            _ => "message",
                        })
                        .unwrap_or("message")
                        .into()
                } else {
                    Event::Message
                };
                // check which callback to use and call it with the data if it's present
                if let Some(function) = self.get_event_callback(&event) {
                    spawn_scoped!({
                        let mut lock = function.1.write().unwrap();
                        // if the data doesn't contain an event type at position `1`, the event must be
                        // of the type `Message`, in that case the data must be on position one and
                        // unwrapping is safe

                        // TODO: refactor
                        lock(
                            Payload::String(
                                contents
                                    .get(1)
                                    .unwrap_or_else(|| contents.get(0).unwrap())
                                    .to_string(),
                            ),
                            SocketIoSocket {
                                socket: self.clone(),
                            },
                        );
                        drop(lock);
                    });
                }
            }
        }
    }

    /// A convenient method for finding a callback for a certain event.
    #[inline]
    fn get_event_callback(&self, event: &Event) -> Option<&(Event, Callback<Payload>)> {
        self.on.iter().find(|item| item.0 == *event)
    }

    fn is_engineio_connected(&self) -> Result<bool> {
        Ok(self.engine_socket.is_connected()?)
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

impl<T: rust_engineio::model::Socket + Debug> Debug for Socket<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("Socket(engine_socket: {:?}, host: {:?}, connected: {:?}, on: <defined callbacks>, outstanding_acks: {:?}, nsp: {:?})",
            self.engine_socket,
            self.host,
            self.connected,
            self.outstanding_acks,
            self.nsp,
        ))
    }
}

pub(crate) struct Iter<'a, EngineSocket: rust_engineio::model::Socket + 'static> {
    socket: &'a Socket<EngineSocket>,
    engine_iter: rust_engineio::client::Iter<'a>,
}

// TODO: change the type to generic
impl<'a> Iterator for Iter<'a, rust_engineio::client::Socket> {
    type Item = Result<SocketPacket>;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        loop {
            let next: std::result::Result<EnginePacket, rust_engineio::Error> =
                self.engine_iter.next()?;

            if let Err(err) = next {
                return Some(Err(err.into()));
            }
            let next = next.unwrap();

            match next.packet_id {
                EnginePacketId::MessageBinary | EnginePacketId::Message => {
                    match self
                        .socket
                        .handle_new_engineio_packet(&mut self.engine_iter, next)
                    {
                        None => {}
                        Some(packet) => return Some(packet),
                    }
                }
                EnginePacketId::Open => {
                    if let Some(function) = self.socket.get_event_callback(&Event::Connect) {
                        let mut lock = function.1.write().unwrap();
                        // TODO: refactor
                        lock(
                            Payload::String(String::from("Connection is opened")),
                            SocketIoSocket {
                                socket: self.socket.clone(),
                            },
                        );
                        drop(lock)
                    }
                }
                EnginePacketId::Close => {
                    if let Some(function) = self.socket.get_event_callback(&Event::Close) {
                        let mut lock = function.1.write().unwrap();
                        // TODO: refactor
                        lock(
                            Payload::String(String::from("Connection is closed")),
                            SocketIoSocket {
                                socket: self.socket.clone(),
                            },
                        );
                        drop(lock)
                    }
                }
                _ => (),
            }
        }
    }
}
