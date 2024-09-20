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
use futures_util::{stream, Stream, StreamExt};
use tokio::{runtime::Handle, sync::Mutex, time::Instant};

use crate::{
    asynchronous::transport::AsyncTransportType,
    error::Result,
    packet::{HandshakePacket, Payload},
    Error, Packet, PacketId,
};
#[cfg(feature = "async-callbacks")]
use super::callback::OptionalCallback;

#[derive(Clone)]
pub struct Socket {
    handle: Handle,
    transport: Arc<Mutex<AsyncTransportType>>,
    transport_raw: AsyncTransportType,
    connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    connection_data: Arc<HandshakePacket>,
    max_ping_timeout: u64,
    // Bellow are the fields that are only used when the async-callbacks feature is enabled.
    #[cfg(feature = "async-callbacks")]
    on_close: OptionalCallback<()>,
    #[cfg(feature = "async-callbacks")]
    on_data: OptionalCallback<Bytes>,
    #[cfg(feature = "async-callbacks")]
    on_error: OptionalCallback<String>,
    #[cfg(feature = "async-callbacks")]
    on_open: OptionalCallback<()>,
    #[cfg(feature = "async-callbacks")]
    on_packet: OptionalCallback<Packet>,
}

impl Socket {
    pub(crate) fn new(
        transport: AsyncTransportType,
        handshake: HandshakePacket,
        #[cfg(feature = "async-callbacks")]
        on_close: OptionalCallback<()>,
        #[cfg(feature = "async-callbacks")]
        on_data: OptionalCallback<Bytes>,
        #[cfg(feature = "async-callbacks")]
        on_error: OptionalCallback<String>,
        #[cfg(feature = "async-callbacks")]
        on_open: OptionalCallback<()>,
        #[cfg(feature = "async-callbacks")]
        on_packet: OptionalCallback<Packet>,
    ) -> Self {
        let max_ping_timeout = handshake.ping_interval + handshake.ping_timeout;

        Socket {
            handle: Handle::current(),
            transport: Arc::new(Mutex::new(transport.clone())),
            transport_raw: transport,
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
            max_ping_timeout,
            #[cfg(feature = "async-callbacks")]
            on_close,
            #[cfg(feature = "async-callbacks")]
            on_data,
            #[cfg(feature = "async-callbacks")]
            on_error,
            #[cfg(feature = "async-callbacks")]
            on_open,
            #[cfg(feature = "async-callbacks")]
            on_packet,
        }
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub async fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        #[cfg(feature = "async-callbacks")]
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

    /// A helper method that distributes the incoming packets to the appropriate callbacks.
    pub(super) async fn handle_incoming_packet(&self, packet: Packet) -> Result<()> {
        // check for the appropriate action or callback
        #[cfg(feature = "async-callbacks")]
        self.handle_packet(packet.clone());
        match packet.packet_id {
            PacketId::MessageBinary => {
                #[cfg(feature = "async-callbacks")]
                self.handle_data(packet.data.clone());
            }
            PacketId::Message => {
                #[cfg(feature = "async-callbacks")]
                self.handle_data(packet.data.clone());
            }
            PacketId::Close => {
                #[cfg(feature = "async-callbacks")]
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
        #[cfg(feature = "async-callbacks")]
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
    #[cfg(feature = "async-callbacks")]
    fn call_error_callback(&self, text: String) {
        if let Some(on_error) = self.on_error.as_ref() {
            let on_error = on_error.clone();
            self.handle.spawn(async move { on_error(text).await });
        }
    }

    #[cfg(not(feature = "async-callbacks"))]
    fn call_error_callback(&self, _text: String) {}

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    pub(crate) async fn pinged(&self) {
        *self.last_ping.lock().await = Instant::now();
    }

    /// Returns the time in milliseconds that is left until a new ping must be received.
    /// This is used to detect whether we have been disconnected from the server.
    /// See https://socket.io/docs/v4/how-it-works/#disconnection-detection
    async fn time_to_next_ping(&self) -> u64 {
        match Instant::now().checked_duration_since(*self.last_ping.lock().await) {
            Some(since_last_ping) => {
                let since_last_ping = since_last_ping.as_millis() as u64;
                if since_last_ping > self.max_ping_timeout {
                    0
                } else {
                    self.max_ping_timeout - since_last_ping
                }
            }
            None => 0,
        }
    }

    #[cfg(feature = "async-callbacks")]
    pub(crate) fn handle_packet(&self, packet: Packet) {
        if let Some(on_packet) = self.on_packet.as_ref() {
            let on_packet = on_packet.clone();
            self.handle.spawn(async move { on_packet(packet).await });
        }
    }

    #[cfg(feature = "async-callbacks")]
    pub(crate) fn handle_data(&self, data: Bytes) {
        if let Some(on_data) = self.on_data.as_ref() {
            let on_data = on_data.clone();
            self.handle.spawn(async move { on_data(data).await });
        }
    }

    #[cfg(feature = "async-callbacks")]
    pub(crate) fn handle_close(&self) {
        if let Some(on_close) = self.on_close.as_ref() {
            let on_close = on_close.clone();
            self.handle.spawn(async move { on_close(()).await });
        }

        self.connected.store(false, Ordering::Release);
    }

    /// Returns the packet stream for the client.
    pub(crate) fn as_stream<'a>(
        &'a self,
    ) -> Pin<Box<dyn Stream<Item = Result<Packet>> + Send + 'a>> {
        stream::unfold(
            Self::stream(self.transport_raw.clone()),
            |mut stream| async {
                // Wait for the next payload or until we should have received the next ping.
                match tokio::time::timeout(
                    std::time::Duration::from_millis(self.time_to_next_ping().await),
                    stream.next(),
                )
                .await
                {
                    Ok(result) => result.map(|result| (result, stream)),
                    // We didn't receive a ping in time and now consider the connection as closed.
                    Err(_) => {
                        // Be nice and disconnect properly.
                        if let Err(e) = self.disconnect().await {
                            Some((Err(e), stream))
                        } else {
                            Some((Err(Error::PingTimeout()), stream))
                        }
                    }
                }
            },
        )
        .boxed()
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = f.debug_struct("Socket");
        let debug = debug
            .field("transport", &self.transport)
            .field("connected", &self.connected)
            .field("last_ping", &self.last_ping)
            .field("last_pong", &self.last_pong)
            .field("connection_data", &self.connection_data);

        #[cfg(feature = "async-callbacks")]
        debug
            .field("on_close", &self.on_close)
            .field("on_data", &self.on_data)
            .field("on_error", &self.on_error)
            .field("on_open", &self.on_open)
            .field("on_packet", &self.on_packet);
        
        debug.finish()
    }
}
