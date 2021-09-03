use crate::callback::OptionalCallback;
use crate::model::Socket;
use crate::transport::TransportType;

use crate::error::{Error, Result};
use crate::packet::{HandshakePacket, Packet, PacketId, Payload};
use bytes::Bytes;
use std::convert::TryFrom;
use std::sync::RwLock;
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Instant,
};

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone)]
pub struct InnerSocket {
    transport: Arc<TransportType>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_packet: OptionalCallback<Packet>,
    connected: Arc<AtomicBool>,
    last_heartbeat: Arc<RwLock<Instant>>,
    connection_data: Arc<HandshakePacket>,
}

impl InnerSocket {
    pub(crate) fn new(
        transport: TransportType,
        handshake: HandshakePacket,
        heartbeat: Instant,
        on_close: OptionalCallback<()>,
        on_data: OptionalCallback<Bytes>,
        on_error: OptionalCallback<String>,
        on_open: OptionalCallback<()>,
        on_packet: OptionalCallback<Packet>,
    ) -> Self {
        InnerSocket {
            on_close,
            on_data,
            on_error,
            on_open,
            on_packet,
            transport: Arc::new(transport),
            connected: Arc::new(AtomicBool::default()),
            connection_data: Arc::new(handshake),
            last_heartbeat: Arc::new(RwLock::new(heartbeat)),
        }
    }

    /// Polls for next payload
    pub(crate) fn poll(&self) -> Result<Option<Payload>> {
        if self.connected.load(Ordering::Acquire) {
            let data = self.transport.as_transport().poll()?;

            if data.is_empty() {
                return Ok(None);
            }

            let payload = Payload::try_from(data)?;

            let iter = payload.iter();

            for packet in iter {
                // check for the appropriate action or callback
                self.handle_packet(packet.clone())?;
                match packet.packet_id {
                    PacketId::MessageBinary => {
                        self.handle_data(packet.data.clone())?;
                    }
                    PacketId::Message => {
                        self.handle_data(packet.data.clone())?;
                    }
                    PacketId::Close => {
                        self.handle_close()?;
                        // set current state to not connected and stop polling
                        self.close()?;
                    }
                    PacketId::Open => {
                        unreachable!("Won't happen as we open the connection beforehand");
                    }
                    _ => {
                        // Handle in client/server implementations
                    }
                }
            }
            Ok(Some(payload))
        } else {
            Err(Error::IllegalActionBeforeOpen())
        }
    }

    pub(crate) fn heartbeat(&self) -> Result<()> {
        *self.last_heartbeat.write()? = Instant::now();
        Ok(())
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) -> Result<()> {
        if let Some(function) = self.on_error.as_ref() {
            spawn_scoped!(function(text));
        }
        Ok(())
    }

    fn handle_packet(&self, packet: Packet) -> Result<()> {
        if let Some(on_packet) = self.on_packet.as_ref() {
            spawn_scoped!(on_packet(packet));
        }
        Ok(())
    }

    fn handle_data(&self, data: Bytes) -> Result<()> {
        if let Some(on_data) = self.on_data.as_ref() {
            spawn_scoped!(on_data(data));
        }
        Ok(())
    }

    fn handle_close(&self) -> Result<()> {
        if let Some(on_close) = self.on_close.as_ref() {
            spawn_scoped!(on_close(()));
        }
        Ok(())
    }
}
impl crate::model::Socket for InnerSocket {
    fn close(&self) -> Result<()> {
        self.emit(Packet::new(PacketId::Close, Bytes::new()))?;
        self.connected.store(false, Ordering::Release);
        Ok(())
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.as_ref() {
            spawn_scoped!(on_open(()));
        }

        // set the last ping to now and set the connected state
        self.heartbeat()?;

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new()))?;

        Ok(())
    }

    /// Sends a packet to the server.
    fn emit(&self, packet: Packet) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }

        let is_binary = packet.packet_id == PacketId::MessageBinary;

        // send a post request with the encoded payload as body
        // if this is a binary attachment, then send the raw bytes
        let data: Bytes = if is_binary {
            packet.data
        } else {
            packet.into()
        };

        if let Err(error) = self.transport.as_transport().emit(data, is_binary) {
            self.call_error_callback(error.to_string())?;
            return Err(error);
        }

        Ok(())
    }

    // Check if the underlying transport client is connected.
    fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }
}

impl Debug for InnerSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocketCommon(transport: {:?}, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_heartbeat: {:?}, connection_data: {:?})",
            self.transport,
            self.on_error,
            self.on_open,
            self.on_close,
            self.on_packet,
            self.on_data,
            self.connected,
            self.last_heartbeat,
            self.connection_data,
        ))
    }
}
