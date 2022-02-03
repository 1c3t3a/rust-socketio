use std::{fmt::Debug, future::Future, pin::Pin, sync::Arc, task::Poll};

use crate::{asynchronous::transport::AsyncTransport, error::Result};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{ready, Stream, StreamExt};
use http::HeaderMap;
use native_tls::TlsConnector;
use tokio::sync::RwLock;
use tokio_tungstenite::{connect_async_tls_with_config, Connector};
use url::Url;

use super::websocket_general::AsyncWebsocketGeneralTransport;

/// An asynchronous websocket transport type.
/// This type only allows for secure websocket
/// connections ("wss://").
pub struct WebsocketSecureTransport {
    inner: AsyncWebsocketGeneralTransport,
    base_url: Arc<RwLock<Url>>,
}

impl WebsocketSecureTransport {
    /// Creates a new instance over a request that might hold additional headers, a possible
    /// Tls connector and an URL.
    pub(crate) async fn new(
        base_url: Url,
        tls_config: Option<TlsConnector>,
        headers: Option<HeaderMap>,
    ) -> Result<Self> {
        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();

        let mut req = http::Request::builder().uri(url.clone().as_str());
        if let Some(map) = headers {
            // SAFETY: this unwrap never panics as the underlying request is just initialized and in proper state
            req.headers_mut().unwrap().extend(map);
        }

        let (ws_stream, _) = connect_async_tls_with_config(
            req.body(())?,
            None,
            tls_config.map(Connector::NativeTls),
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
}

impl Stream for WebsocketSecureTransport {
    type Item = Result<Bytes>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let data = ready!(Pin::new(&mut Box::pin(self.inner.poll())).poll(cx))?;

        Poll::Ready(Some(Ok(data)))
    }
}

#[async_trait]
impl AsyncTransport for WebsocketSecureTransport {
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        self.inner.emit(data, is_binary_att).await
    }

    async fn poll(&self) -> Result<Bytes> {
        self.inner.poll().await
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
