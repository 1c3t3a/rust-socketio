use std::{
    fmt::Debug,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use bytes::Bytes;
use tokio::{
    sync::{Mutex, RwLock},
    time::Instant,
};

use crate::{
    asynchronous::{callback::OptionalCallback, transport::AsyncTransportType},
    error::Result,
    packet::{HandshakePacket, Payload},
    Error, Packet, PacketId,
};

#[derive(Clone)]
pub struct Socket {
    transport: Arc<AsyncTransportType>,
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
}

impl Socket {
    pub(crate) fn new(
        transport: AsyncTransportType,
        handshake: HandshakePacket,
        on_close: OptionalCallback<()>,
        on_data: OptionalCallback<Bytes>,
        on_error: OptionalCallback<String>,
        on_open: OptionalCallback<()>,
        on_packet: OptionalCallback<Packet>,
    ) -> Self {
        Socket {
            on_close,
            on_data,
            on_error,
            on_open,
            on_packet,
            transport: Arc::new(transport),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
            remaining_packets: Arc::new(RwLock::new(None)),
        }
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub async fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.as_ref() {
            // TODO: Check how this can be spawned in a runtime, we don't want to
            // block we only want to execute it somewhere
            on_open(()).await;
        }

        // set the last ping to now and set the connected state
        *self.last_ping.lock().await = Instant::now();

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new())).await?;

        Ok(())
    }

    /// Polls for next payload
    pub(crate) async fn poll(&self) -> Result<Option<Packet>> {
        loop {
            if self.connected.load(Ordering::Acquire) {
                if self.remaining_packets.read().await.is_some() {
                    // SAFETY: checked is some above
                    let mut iter = self.remaining_packets.write().await;
                    let iter = iter.as_mut().unwrap();
                    if let Some(packet) = iter.next() {
                        return Ok(Some(packet));
                    }
                }

                // Iterator has run out of packets, get a new payload
                let data = self.transport.as_transport().poll().await?;

                if data.is_empty() {
                    continue;
                }

                let payload = Payload::try_from(data)?;
                let mut iter = payload.into_iter();

                if let Some(packet) = iter.next() {
                    *self.remaining_packets.write().await = Some(iter);
                    return Ok(Some(packet));
                }
            } else {
                return Ok(None);
            }
        }
    }

    pub async fn disconnect(&self) -> Result<()> {
        if let Some(on_close) = self.on_close.as_ref() {
            on_close(()).await;
        }

        self.emit(Packet::new(PacketId::Close, Bytes::new()))
            .await?;

        self.connected.store(false, Ordering::Release);

        Ok(())
    }

    /// Sends a packet to the server.
    pub async fn emit(&self, packet: Packet) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            // TODO: Check how this can be spawned in a runtime, we don't want to
            // block we only want to execute it somewhere
            self.call_error_callback(format!("{}", error)).await;
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

        if let Err(error) = self.transport.as_transport().emit(data, is_binary).await {
            // TODO: Check how this can be spawned in a runtime, we don't want to
            // block we only want to execute it somewhere
            self.call_error_callback(error.to_string()).await;
            return Err(error);
        }

        Ok(())
    }

    /// Calls the error callback with a given message.
    #[inline]
    async fn call_error_callback(&self, text: String) {
        if let Some(function) = self.on_error.as_ref() {
            function(text).await;
        }
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }

    pub(crate) async fn pinged(&self) -> Result<()> {
        *self.last_ping.lock().await = Instant::now();
        Ok(())
    }

    pub(crate) async fn handle_packet(&self, packet: Packet) {
        if let Some(on_packet) = self.on_packet.as_ref() {
            on_packet(packet).await;
        }
    }

    pub(crate) async fn handle_data(&self, data: Bytes) {
        if let Some(on_data) = self.on_data.as_ref() {
            on_data(data).await;
        }
    }

    pub(crate) async fn handle_close(&self) {
        if let Some(on_close) = self.on_close.as_ref() {
            on_close(()).await;
        }

        self.connected.store(false, Ordering::Release);
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Socket")
            .field("transport", &self.transport)
            .field("on_close", &self.on_close)
            .field("on_data", &self.on_data)
            .field("on_error", &self.on_error)
            .field("on_open", &self.on_open)
            .field("on_packet", &self.on_packet)
            .field("connected", &self.connected)
            .field("last_ping", &self.last_ping)
            .field("last_pong", &self.last_pong)
            .field("connection_data", &self.connection_data)
            .field("remaining_packets", &self.remaining_packets)
            .finish()
    }
}
