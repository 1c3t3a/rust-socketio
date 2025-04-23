use std::fmt::Debug;
use std::pin::Pin;
use std::sync::Arc;

use crate::asynchronous::transport::AsyncTransport;
use crate::error::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;
use futures_util::StreamExt;
use http::HeaderMap;
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::Connector;
use tungstenite::client::IntoClientRequest;
use url::Url;

use super::websocket_general::AsyncWebsocketGeneralTransport;
use crate::TlsConfig;

/// An asynchronous websocket transport type.
/// This type only allows for secure websocket
/// connections ("wss://").
#[derive(Clone)]
pub struct WebsocketSecureTransport {
    inner: AsyncWebsocketGeneralTransport,
    base_url: Arc<RwLock<Url>>,
}

impl WebsocketSecureTransport {
    /// Creates a new instance over a request that might hold additional headers, a possible
    /// Tls connector and an URL.
    pub(crate) async fn new(
        base_url: Url,
        tls_config: Option<TlsConfig>,
        headers: Option<HeaderMap>,
    ) -> Result<Self> {
        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();

        let mut req = url.clone().into_client_request()?;
        if let Some(map) = headers {
            // SAFETY: this unwrap never panics as the underlying request is just initialized and in proper state
            req.headers_mut().extend(map);
        }

        // `disable_nagle` Sets the value of the TCP_NODELAY option on this socket.
        //
        // If set to `true`, this option disables the Nagle algorithm.
        // This means that segments are always sent as soon as possible, even if there is only a small amount of data.
        // When `false`, data is buffered until there is a sufficient amount to send out, thereby avoiding the frequent sending of small packets.
        //
        // See the docs: https://docs.rs/tokio/latest/tokio/net/struct.TcpStream.html#method.set_nodelay
        let (ws_stream, _) = connect_async_tls_with_config(
            req,
            None,
            /*disable_nagle=*/ false,
            #[cfg(all(feature = "_native-tls", not(feature = "_rustls-tls")))]
            tls_config.map(Connector::NativeTls),
            #[cfg(feature = "_rustls-tls")]
            tls_config.map(Arc::new).map(Connector::Rustls),
            #[cfg(not(any(feature = "_native-tls", feature = "_rustls-tls")))]
            compile_error!("Either feature `_native-tls` or `_rustls-tls` must be enabled"),
        )
        .await?;

        let (sen, rec) = ws_stream.split();
        let inner = AsyncWebsocketGeneralTransport::new(sen, rec).await;

        Ok(WebsocketSecureTransport {
            inner,
            base_url: Arc::new(RwLock::new(url)),
        })
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) async fn upgrade(&self) -> Result<()> {
        self.inner.upgrade().await
    }

    pub(crate) async fn poll_next(&self) -> Result<Option<Bytes>> {
        self.inner.poll_next().await
    }
}

impl Stream for WebsocketSecureTransport {
    type Item = Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

#[async_trait]
impl AsyncTransport for WebsocketSecureTransport {
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        self.inner.emit(data, is_binary_att).await
    }

    async fn base_url(&self) -> Result<Url> {
        Ok(self.base_url.read().await.clone())
    }

    async fn set_base_url(&self, base_url: Url) -> Result<()> {
        let mut url = base_url;
        if !url
            .query_pairs()
            .any(|(k, v)| k == "transport" && v == "websocket")
        {
            url.query_pairs_mut().append_pair("transport", "websocket");
        }
        url.set_scheme("wss").unwrap();
        *self.base_url.write().await = url;
        Ok(())
    }
}

impl Debug for WebsocketSecureTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncWebsocketSecureTransport")
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

    async fn new() -> Result<WebsocketSecureTransport> {
        let url = crate::test::engine_io_server_secure()?.to_string()
            + "engine.io/?EIO="
            + &ENGINE_IO_VERSION.to_string();
        WebsocketSecureTransport::new(
            Url::from_str(&url[..])?,
            Some(crate::test::tls_connector()?),
            None,
        )
        .await
    }

    #[tokio::test]
    async fn websocket_secure_transport_base_url() -> Result<()> {
        let transport = new().await?;
        let mut url = crate::test::engine_io_server_secure()?;
        url.set_path("/engine.io/");
        url.query_pairs_mut()
            .append_pair("EIO", &ENGINE_IO_VERSION.to_string())
            .append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();
        assert_eq!(transport.base_url().await?.to_string(), url.to_string());
        transport
            .set_base_url(reqwest::Url::parse("https://127.0.0.1")?)
            .await?;
        assert_eq!(
            transport.base_url().await?.to_string(),
            "wss://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url().await?.to_string(), url.to_string());

        transport
            .set_base_url(reqwest::Url::parse(
                "http://127.0.0.1/?transport=websocket",
            )?)
            .await?;
        assert_eq!(
            transport.base_url().await?.to_string(),
            "wss://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url().await?.to_string(), url.to_string());
        Ok(())
    }

    #[tokio::test]
    async fn websocket_secure_debug() -> Result<()> {
        let transport = new().await?;
        assert_eq!(
            format!("{:?}", transport),
            format!(
                "AsyncWebsocketSecureTransport {{ base_url: {:?} }}",
                transport.base_url().await?.to_string()
            )
        );
        Ok(())
    }
}
