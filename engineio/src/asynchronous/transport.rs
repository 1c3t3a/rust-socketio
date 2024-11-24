use crate::error::Result;
use adler32::adler32;
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::Stream;
use std::{pin::Pin, time::SystemTime};
use url::Url;

use super::async_transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};

#[async_trait]
pub trait AsyncTransport: Stream<Item = Result<Bytes>> + Unpin {
    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()>;

    /// Returns start of the url. ex. http://localhost:2998/engine.io/?EIO=4&transport=polling
    /// Must have EIO and transport already set.
    async fn base_url(&self) -> Result<Url>;

    /// Used to update the base path, like when adding the sid.
    async fn set_base_url(&self, base_url: Url) -> Result<()>;

    /// Full query address
    async fn address(&self) -> Result<Url>
    where
        Self: Sized,
    {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        let mut url = self.base_url().await?;
        url.query_pairs_mut().append_pair("t", &hash.to_string());
        Ok(url)
    }
}

#[derive(Debug, Clone)]
pub enum AsyncTransportType {
    Polling(PollingTransport),
    Websocket(WebsocketTransport),
    WebsocketSecure(WebsocketSecureTransport),
}

impl From<PollingTransport> for AsyncTransportType {
    fn from(transport: PollingTransport) -> Self {
        AsyncTransportType::Polling(transport)
    }
}

impl From<WebsocketTransport> for AsyncTransportType {
    fn from(transport: WebsocketTransport) -> Self {
        AsyncTransportType::Websocket(transport)
    }
}

impl From<WebsocketSecureTransport> for AsyncTransportType {
    fn from(transport: WebsocketSecureTransport) -> Self {
        AsyncTransportType::WebsocketSecure(transport)
    }
}

impl AsyncTransportType {
    pub fn as_transport(&self) -> &(dyn AsyncTransport + Send) {
        match self {
            AsyncTransportType::Polling(transport) => transport,
            AsyncTransportType::Websocket(transport) => transport,
            AsyncTransportType::WebsocketSecure(transport) => transport,
        }
    }

    pub fn as_pin_box(&mut self) -> Pin<Box<&mut (dyn AsyncTransport + Send)>> {
        match self {
            AsyncTransportType::Polling(transport) => Box::pin(transport),
            AsyncTransportType::Websocket(transport) => Box::pin(transport),
            AsyncTransportType::WebsocketSecure(transport) => Box::pin(transport),
        }
    }
}
