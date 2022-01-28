use std::sync::Arc;

use crate::error::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::StreamExt;
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async;
use url::Url;

use super::transport::AsyncTransport;
use super::AsyncWebsocketGeneralTransport;

/// An asynchronous websocket transport type.
/// This type only allows for plain websocket
/// connections ("ws://").
pub(crate) struct AsyncWebsocketTransport {
    inner: AsyncWebsocketGeneralTransport,
    base_url: Arc<RwLock<Url>>,
}

impl AsyncWebsocketTransport {
    /// Creates a new instance over a request that might hold additional headers and an URL.
    pub async fn new(request: http::request::Request<()>, url: Url) -> Result<Self> {
        let (ws_stream, _) = connect_async(request).await?;
        let (sen, rec) = ws_stream.split();

        let inner = AsyncWebsocketGeneralTransport::new(sen, rec).await;
        Ok(AsyncWebsocketTransport {
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

#[async_trait]
impl AsyncTransport for AsyncWebsocketTransport {
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
        url.set_scheme("ws").unwrap();
        *self.base_url.write().await = url;
        Ok(())
    }
}
