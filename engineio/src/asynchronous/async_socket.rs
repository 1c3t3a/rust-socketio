use std::{
    fmt::Debug,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_stream::try_stream;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use tokio::{runtime::Handle, sync::Mutex, time::Instant};

use crate::{
    asynchronous::{callback::OptionalCallback, transport::AsyncTransportType},
    error::Result,
    packet::{HandshakePacket, Payload},
    Error, Packet, PacketId,
};

use super::generator::StreamGenerator;

#[derive(Clone)]
pub struct Socket {
    handle: Handle,
    transport: Arc<Mutex<AsyncTransportType>>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_packet: OptionalCallback<Packet>,
    connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    connection_data: Arc<HandshakePacket>,
    generator: StreamGenerator<Packet>,
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
            handle: Handle::current(),
            on_close,
            on_data,
            on_error,
            on_open,
            on_packet,
            transport: Arc::new(Mutex::new(transport.clone())),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
            generator: StreamGenerator::new(Self::stream(transport)),
        }
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub async fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.as_ref() {
            let on_open = on_open.clone();
            self.handle.spawn(async move { on_open(()).await });
        }

        // set the last ping to now and set the connected state
        *self.last_ping.lock().await = Instant::now();

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new())).await?;

        Ok(())
    }

    /// A helper method that distributes
    pub(super) async fn handle_inconming_packet(&self, packet: Packet) -> Result<()> {
        // check for the appropriate action or callback
        self.handle_packet(packet.clone());
        match packet.packet_id {
            PacketId::MessageBinary => {
                self.handle_data(packet.data.clone());
            }
            PacketId::Message => {
                self.handle_data(packet.data.clone());
            }
            PacketId::Close => {
                self.handle_close();
            }
            PacketId::Upgrade => {
                // this is already checked during the handshake, so just do nothing here
            }
            PacketId::Ping => {
                self.pinged().await;
                self.emit(Packet::new(PacketId::Pong, Bytes::new())).await?;
            }
            PacketId::Pong | PacketId::Open => {
                // this will never happen as the pong and open
                // packets are only sent by the client
                return Err(Error::InvalidPacket());
            }
            PacketId::Noop => (),
        }
        Ok(())
    }

    /// Helper method that parses bytes and returns an iterator over the elements.
    fn parse_payload(bytes: Bytes) -> impl Stream<Item = Result<Packet>> {
        try_stream! {
            let payload = Payload::try_from(bytes);

            for elem in payload?.into_iter() {
                yield elem;
            }
        }
    }

    /// Creates a stream over the incoming packets, uses the streams provided by the
    /// underlying transport types.
    fn stream(
        mut transport: AsyncTransportType,
    ) -> Pin<Box<impl Stream<Item = Result<Packet>> + 'static + Send>> {
        // map the byte stream of the underlying transport
        // to a packet stream
        Box::pin(try_stream! {
            for await payload in transport.as_pin_box() {
                for await packet in Self::parse_payload(payload?) {
                    yield packet?;
                }
            }
        })
    }

    pub async fn disconnect(&self) -> Result<()> {
        if let Some(on_close) = self.on_close.as_ref() {
            let on_close = on_close.clone();
            self.handle.spawn(async move { on_close(()).await });
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
            self.call_error_callback(format!("{}", error));
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

        let lock = self.transport.lock().await;
        let fut = lock.as_transport().emit(data, is_binary);

        if let Err(error) = fut.await {
            self.call_error_callback(error.to_string());
            return Err(error);
        }

        Ok(())
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) {
        if let Some(on_error) = self.on_error.as_ref() {
            let on_error = on_error.clone();
            self.handle.spawn(async move { on_error(text).await });
        }
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub(crate) async fn pinged(&self) {
        *self.last_ping.lock().await = Instant::now();
    }

    pub(crate) fn handle_packet(&self, packet: Packet) {
        if let Some(on_packet) = self.on_packet.as_ref() {
            let on_packet = on_packet.clone();
            self.handle.spawn(async move { on_packet(packet).await });
        }
    }

    pub(crate) fn handle_data(&self, data: Bytes) {
        if let Some(on_data) = self.on_data.as_ref() {
            let on_data = on_data.clone();
            self.handle.spawn(async move { on_data(data).await });
        }
    }

    pub(crate) fn handle_close(&self) {
        if let Some(on_close) = self.on_close.as_ref() {
            let on_close = on_close.clone();
            self.handle.spawn(async move { on_close(()).await });
        }

        self.connected.store(false, Ordering::Release);
    }
}

impl Stream for Socket {
    type Item = Result<Packet>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.generator.poll_next_unpin(cx)
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
            .finish()
    }
}
