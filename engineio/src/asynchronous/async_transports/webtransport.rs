//! WebTransport transport implementation for Engine.IO
//!
//! This module provides WebTransport support as an alternative to WebSocket
//! for Engine.IO connections. WebTransport offers lower latency and supports
//! both reliable streams and unreliable datagrams over HTTP/3.

use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;

use crate::asynchronous::transport::AsyncTransport;
use crate::error::Result;
use crate::{Error, Packet, PacketId};
use async_stream::try_stream;
use async_trait::async_trait;
use base64::Engine as Base64Engine;
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{ready, FutureExt, Stream};
use http::HeaderMap;
use tokio::sync::{Mutex, RwLock};
use url::Url;
use wtransport::{ClientConfig, Endpoint, Connection, RecvStream, SendStream};

/// Internal generator type for the receive stream
type StreamGenerator = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send + Sync>>;

/// An asynchronous WebTransport transport type.
/// Uses HTTP/3 with QUIC for low-latency bidirectional communication.
pub struct WebTransportTransport {
    connection: Arc<Connection>,
    sender: Arc<Mutex<SendStream>>,
    receiver: Arc<Mutex<RecvStream>>,
    generator: Arc<Mutex<StreamGenerator>>,
    base_url: Arc<RwLock<Url>>,
}

/// Creates a stream generator from a RecvStream
fn create_stream_generator(receiver: Arc<Mutex<RecvStream>>) -> StreamGenerator {
    Box::pin(try_stream! {
        loop {
            let mut guard = receiver.lock().await;
            let mut buf = vec![0u8; 4096];
            match guard.read(&mut buf).await {
                Ok(Some(n)) if n > 0 => {
                    buf.truncate(n);
                    yield Bytes::from(buf);
                }
                Ok(_) => break,
                Err(e) => {
                    Err(e)?;
                    break;
                }
            }
        }
    })
}

impl WebTransportTransport {
    /// Creates a new WebTransport connection to the given URL.
    ///
    /// # Arguments
    /// * `base_url` - The URL to connect to (https:// scheme)
    /// * `headers` - Optional HTTP headers (note: WebTransport has limited header support)
    pub async fn new(base_url: Url, _headers: Option<HeaderMap>) -> Result<Self> {
        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "webtransport");

        // Ensure HTTPS scheme for WebTransport
        if url.scheme() != "https" {
            url.set_scheme("https").map_err(|_| Error::InvalidUrlScheme("WebTransport requires https".to_string()))?;
        }

        // Create WebTransport client config
        // For development, we skip certificate verification
        // In production, proper certificate validation should be used
        let config = ClientConfig::builder()
            .with_bind_default()
            .with_no_cert_validation()
            .build();

        let endpoint = Endpoint::client(config)?;

        // Connect to the WebTransport server
        let connection = endpoint.connect(&url.to_string()).await?;

        // Open a bidirectional stream for Engine.IO communication
        let (send_stream, recv_stream) = connection.open_bi().await?.await?;

        let receiver = Arc::new(Mutex::new(recv_stream));
        let generator = create_stream_generator(Arc::clone(&receiver));

        Ok(WebTransportTransport {
            connection: Arc::new(connection),
            sender: Arc::new(Mutex::new(send_stream)),
            receiver,
            generator: Arc::new(Mutex::new(generator)),
            base_url: Arc::new(RwLock::new(url)),
        })
    }

    /// Creates a new WebTransport connection with custom TLS configuration.
    ///
    /// # Arguments
    /// * `base_url` - The URL to connect to
    /// * `config` - Custom WebTransport client configuration
    pub async fn with_config(base_url: Url, config: ClientConfig) -> Result<Self> {
        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "webtransport");

        if url.scheme() != "https" {
            url.set_scheme("https").map_err(|_| Error::InvalidUrlScheme("WebTransport requires https".to_string()))?;
        }

        let endpoint = Endpoint::client(config)?;
        let connection = endpoint.connect(&url.to_string()).await?;
        let (send_stream, recv_stream) = connection.open_bi().await?.await?;

        let receiver = Arc::new(Mutex::new(recv_stream));
        let generator = create_stream_generator(Arc::clone(&receiver));

        Ok(WebTransportTransport {
            connection: Arc::new(connection),
            sender: Arc::new(Mutex::new(send_stream)),
            receiver,
            generator: Arc::new(Mutex::new(generator)),
            base_url: Arc::new(RwLock::new(url)),
        })
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade request.
    /// Used when upgrading from polling to WebTransport.
    pub(crate) async fn upgrade(&self) -> Result<()> {
        let mut sender = self.sender.lock().await;
        let mut receiver = self.receiver.lock().await;

        // Send probe packet
        let probe_packet = Packet::new(PacketId::Ping, Bytes::from("probe"));
        let probe_bytes = Bytes::from(probe_packet);
        sender.write_all(&probe_bytes).await?;

        // Read response
        let mut buf = vec![0u8; 1024];
        let n = receiver.read(&mut buf).await?.ok_or(Error::IncompletePacket())?;
        let response = Bytes::copy_from_slice(&buf[..n]);

        // Verify probe response
        let expected = Bytes::from(Packet::new(PacketId::Pong, Bytes::from("probe")));
        if response != expected {
            return Err(Error::InvalidPacket());
        }

        // Send upgrade packet
        let upgrade_packet = Packet::new(PacketId::Upgrade, Bytes::from(""));
        let upgrade_bytes = Bytes::from(upgrade_packet);
        sender.write_all(&upgrade_bytes).await?;

        Ok(())
    }

    /// Polls for the next message from the WebTransport stream.
    pub(crate) async fn poll_next(&self) -> Result<Option<Bytes>> {
        let mut receiver = self.receiver.lock().await;

        // Read into a buffer
        let mut buf = vec![0u8; 4096];
        match receiver.read(&mut buf).await? {
            Some(n) if n > 0 => {
                buf.truncate(n);
                Ok(Some(Bytes::from(buf)))
            }
            _ => Ok(None),
        }
    }

    /// Gets the underlying connection for advanced operations like datagrams.
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Sends an unreliable datagram (for audio/video streaming).
    /// Datagrams may be lost but have lower latency than streams.
    pub async fn send_datagram(&self, data: &[u8]) -> Result<()> {
        self.connection.send_datagram(data)?;
        Ok(())
    }

    /// Receives an unreliable datagram.
    pub async fn receive_datagram(&self) -> Result<Bytes> {
        let datagram = self.connection.receive_datagram().await?;
        Ok(Bytes::from(datagram.to_vec()))
    }
}

#[async_trait]
impl AsyncTransport for WebTransportTransport {
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut sender = self.sender.lock().await;

        let message = if is_binary_att {
            // For binary attachments, prefix with 'b' and base64 encode
            let encoded = base64::engine::general_purpose::STANDARD.encode(&data);
            let mut msg = BytesMut::with_capacity(encoded.len() + 1);
            msg.put_u8(b'b');
            msg.put(encoded.as_bytes());
            msg.freeze()
        } else {
            data
        };

        sender.write_all(&message).await?;
        Ok(())
    }

    async fn base_url(&self) -> Result<Url> {
        Ok(self.base_url.read().await.clone())
    }

    async fn set_base_url(&self, base_url: Url) -> Result<()> {
        let mut url = base_url;
        if !url
            .query_pairs()
            .any(|(k, v)| k == "transport" && v == "webtransport")
        {
            url.query_pairs_mut().append_pair("transport", "webtransport");
        }
        if url.scheme() != "https" {
            url.set_scheme("https").map_err(|_| Error::InvalidUrlScheme("WebTransport requires https".to_string()))?;
        }
        *self.base_url.write().await = url;
        Ok(())
    }
}

impl Stream for WebTransportTransport {
    type Item = Result<Bytes>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        // Lock the generator and poll it
        let mut lock = ready!(Box::pin(self.generator.lock()).poll_unpin(cx));
        lock.as_mut().poll_next(cx)
    }
}

impl Clone for WebTransportTransport {
    fn clone(&self) -> Self {
        // Create a new generator from the shared receiver
        let generator = create_stream_generator(Arc::clone(&self.receiver));
        WebTransportTransport {
            connection: Arc::clone(&self.connection),
            sender: Arc::clone(&self.sender),
            receiver: Arc::clone(&self.receiver),
            generator: Arc::new(Mutex::new(generator)),
            base_url: Arc::clone(&self.base_url),
        }
    }
}

impl Debug for WebTransportTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebTransportTransport")
            .field(
                "base_url",
                &self
                    .base_url
                    .try_read()
                    .map_or("Currently not available".to_owned(), |url| url.to_string()),
            )
            .finish()
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ENGINE_IO_VERSION;
    use std::str::FromStr;

    // Note: These tests require a WebTransport-capable server
    // The test server should support the Engine.IO protocol over WebTransport

    #[tokio::test]
    #[ignore] // Requires WebTransport server
    async fn webtransport_transport_base_url() -> Result<()> {
        let url = "https://localhost:4433/engine.io/?EIO=".to_string()
            + &ENGINE_IO_VERSION.to_string();
        let transport = WebTransportTransport::new(Url::from_str(&url)?, None).await?;

        let base = transport.base_url().await?;
        assert!(base.query_pairs().any(|(k, v)| k == "transport" && v == "webtransport"));
        assert_eq!(base.scheme(), "https");

        Ok(())
    }

    #[tokio::test]
    #[ignore] // Requires WebTransport server
    async fn webtransport_set_base_url() -> Result<()> {
        let url = "https://localhost:4433/engine.io/?EIO=".to_string()
            + &ENGINE_IO_VERSION.to_string();
        let transport = WebTransportTransport::new(Url::from_str(&url)?, None).await?;

        transport.set_base_url(Url::parse("http://127.0.0.1")?).await?;
        let base = transport.base_url().await?;

        // Should have been converted to https
        assert_eq!(base.scheme(), "https");
        assert!(base.query_pairs().any(|(k, v)| k == "transport" && v == "webtransport"));

        Ok(())
    }
}
