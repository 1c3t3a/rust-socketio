use std::sync::Arc;

use crate::error::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use native_tls::TlsConnector;
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::Connector;
use url::Url;

use super::transport::AsyncTransport;
use super::AsyncWebsocketGeneralTransport;

/// An asynchronous websocket transport type.
/// This type only allows for secure websocket
/// connections ("wss://").
pub(crate) struct AsyncWebsocketSecureTransport {
    inner: AsyncWebsocketGeneralTransport,
    base_url: Arc<RwLock<url::Url>>,
}

impl AsyncWebsocketSecureTransport {
    /// Creates a new instance over a request that might hold additional headers, a possible
    /// Tls connector and an URL.
    pub(crate) async fn new(
        request: http::request::Request<()>,
        base_url: url::Url,
        tls_config: Option<TlsConnector>,
    ) -> Result<Self> {
        let (ws_stream, _) =
            connect_async_tls_with_config(request, None, tls_config.map(Connector::NativeTls))
                .await?;

        let (sen, rec) = ws_stream.split();
        let inner = AsyncWebsocketGeneralTransport::new(sen, rec).await;

        Ok(AsyncWebsocketSecureTransport {
            inner,
            base_url: Arc::new(RwLock::new(base_url)),
        })
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) async fn upgrade(&self) -> Result<()> {
        self.inner.upgrade().await
    }
}

#[async_trait]
impl AsyncTransport for AsyncWebsocketSecureTransport {
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
