use crate::callback::OptionalCallback;
use crate::transport::TransportType;

use crate::error::{Error, Result};
use crate::packet::{HandshakePacket, Packet, PacketId, PacketSerializer};
use bytes::Bytes;
use std::sync::RwLock;
use std::time::Duration;
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex},
    time::Instant,
};

/// The default maximum ping timeout as calculated from the pingInterval and pingTimeout.
/// See https://socket.io/docs/v4/server-options/#pinginterval and
/// https://socket.io/docs/v4/server-options/#pingtimeout
pub const DEFAULT_MAX_POLL_TIMEOUT: Duration = Duration::from_secs(45);

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone)]
pub struct Socket {
    transport: Arc<TransportType>,
    serializer: Arc<PacketSerializer>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_packet: OptionalCallback<Packet>,
    connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    connection_data: Arc<HandshakePacket>,
    /// Since we get packets in payloads it's possible to have a state where only some of the packets have been consumed.
    remaining_packets: Arc<RwLock<Option<crate::packet::IntoIter>>>,
    max_ping_timeout: u64,
}

impl Socket {
    pub(crate) fn new(
        transport: TransportType,
        serializer: Arc<PacketSerializer>,
        handshake: HandshakePacket,
        on_close: OptionalCallback<()>,
        on_data: OptionalCallback<Bytes>,
        on_error: OptionalCallback<String>,
        on_open: OptionalCallback<()>,
        on_packet: OptionalCallback<Packet>,
    ) -> Self {
        let max_ping_timeout = handshake.ping_interval + handshake.ping_timeout;

        Socket {
            on_close,
            on_data,
            on_error,
            on_open,
            on_packet,
            transport: Arc::new(transport),
            serializer,
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
            remaining_packets: Arc::new(RwLock::new(None)),
            max_ping_timeout,
        }
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.as_ref() {
            spawn_scoped!(on_open(()));
        }

        // set the last ping to now and set the connected state
        *self.last_ping.lock()? = Instant::now();

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new()))?;

        Ok(())
    }

    pub fn disconnect(&self) -> Result<()> {
        if let Some(on_close) = self.on_close.as_ref() {
            spawn_scoped!(on_close(()));
        }

        // will not succeed when connection to the server is interrupted
        let _ = self.emit(Packet::new(PacketId::Close, Bytes::new()));

        self.connected.store(false, Ordering::Release);

        Ok(())
    }

    /// Sends a packet to the server.
    pub fn emit(&self, packet: Packet) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error));
            return Err(error);
        }

        let is_binary = packet.packet_id == PacketId::MessageBinary;

        // send a post request with the encoded payload as body
        // if this is a binary attachment, then send the raw bytes
        let data: Bytes = if is_binary {
            packet.data
        } else {
            // packet.into()
            self.serializer.encode(packet)
        };

        if let Err(error) = self.transport.as_transport().emit(data, is_binary) {
            self.call_error_callback(error.to_string());
            return Err(error);
        }

        Ok(())
    }

    /// Polls for next payload
    pub(crate) fn poll(&self) -> Result<Option<Packet>> {
        loop {
            if self.connected.load(Ordering::Acquire) {
                if self.remaining_packets.read()?.is_some() {
                    // SAFETY: checked is some above
                    let mut iter = self.remaining_packets.write()?;
                    let iter = iter.as_mut().unwrap();
                    if let Some(packet) = iter.next() {
                        return Ok(Some(packet));
                    }
                }

                // Iterator has run out of packets, get a new payload.
                // Make sure that payload is received within time_to_next_ping, as otherwise the heart
                // stopped beating and we disconnect.
                let data = self
                    .transport
                    .as_transport()
                    .poll(Duration::from_millis(self.time_to_next_ping()?))?;

                if data.is_empty() {
                    continue;
                }

                // let payload = Payload::try_from(data)?;
                let payload = self.serializer.decode_payload(data)?;
                let mut iter = payload.into_iter();

                if let Some(packet) = iter.next() {
                    *self.remaining_packets.write()? = Some(iter);
                    return Ok(Some(packet));
                }
            } else {
                return Ok(None);
            }
        }
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) {
        if let Some(function) = self.on_error.as_ref() {
            spawn_scoped!(function(text));
        }
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }

    pub(crate) fn pinged(&self) -> Result<()> {
        *self.last_ping.lock()? = Instant::now();
        Ok(())
    }

    /// Returns the time in milliseconds that is left until a new ping must be received.
    /// This is used to detect whether we have been disconnected from the server.
    /// See https://socket.io/docs/v4/how-it-works/#disconnection-detection
    fn time_to_next_ping(&self) -> Result<u64> {
        match Instant::now().checked_duration_since(*self.last_ping.lock()?) {
            Some(since_last_ping) => {
                let since_last_ping = since_last_ping.as_millis() as u64;
                if since_last_ping > self.max_ping_timeout {
                    Ok(0)
                } else {
                    Ok(self.max_ping_timeout - since_last_ping)
                }
            }
            None => Ok(0),
        }
    }

    pub(crate) fn handle_packet(&self, packet: Packet) {
        if let Some(on_packet) = self.on_packet.as_ref() {
            spawn_scoped!(on_packet(packet));
        }
    }

    pub(crate) fn handle_data(&self, data: Bytes) {
        if let Some(on_data) = self.on_data.as_ref() {
            spawn_scoped!(on_data(data));
        }
    }

    pub(crate) fn handle_close(&self) {
        if let Some(on_close) = self.on_close.as_ref() {
            spawn_scoped!(on_close(()));
        }

        self.connected.store(false, Ordering::Release);
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for Socket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocket(transport: {:?}, serializer: {:?}, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_ping: {:?}, last_pong: {:?}, connection_data: {:?})",
            self.transport,
            self.serializer,
            self.on_error,
            self.on_open,
            self.on_close,
            self.on_packet,
            self.on_data,
            self.connected,
            self.last_ping,
            self.last_pong,
            self.connection_data,
        ))
    }
}
