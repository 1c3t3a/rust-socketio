use std::fmt::Debug;
use std::sync::Arc;

use crate::asynchronous::transport::AsyncTransport;
use crate::error::Result;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::stream::StreamExt;
use http::HeaderMap;
use tokio::sync::RwLock;
use tokio_tungstenite::connect_async;
use url::Url;

use super::websocket_general::AsyncWebsocketGeneralTransport;

/// An asynchronous websocket transport type.
/// This type only allows for plain websocket
/// connections ("ws://").
pub struct WebsocketTransport {
    inner: AsyncWebsocketGeneralTransport,
    base_url: Arc<RwLock<Url>>,
}

impl WebsocketTransport {
    /// Creates a new instance over a request that might hold additional headers and an URL.
    pub async fn new(url: Url, headers: Option<HeaderMap>) -> Result<Self> {
        let mut url = url;
        url.query_pairs_mut().append_pair("transport", "websocket");
        url.set_scheme("ws").unwrap();

        let mut req = http::Request::builder().uri(url.clone().as_str());
        if let Some(map) = headers {
            // SAFETY: this unwrap never panics as the underlying request is just initialized and in proper state
            req.headers_mut().unwrap().extend(map);
        }

        let (ws_stream, _) = connect_async(req.body(())?).await?;
        let (sen, rec) = ws_stream.split();

        let inner = AsyncWebsocketGeneralTransport::new(sen, rec).await;
        Ok(WebsocketTransport {
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
impl AsyncTransport for WebsocketTransport {
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

impl Debug for WebsocketTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AsyncWebsocketTransport")
            .field("base_url", &self.base_url)
            .finish()
    }
}
