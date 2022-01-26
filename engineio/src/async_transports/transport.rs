use crate::error::Result;
use adler32::adler32;
use async_trait::async_trait;
use bytes::Bytes;
use std::time::SystemTime;
use url::Url;

#[async_trait]
pub trait AsyncTransport {
    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()>;

    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    async fn poll(&self) -> Result<Bytes>;

    /// Returns start of the url. ex. http://localhost:2998/engine.io/?EIO=4&transport=polling
    /// Must have EIO and transport already set.
    async fn base_url(&self) -> Result<Url>;

    /// Used to update the base path, like when adding the sid.
    async fn set_base_url(&self, base_url: Url) -> Result<()>;

    /// Full query address
    async fn address(&self) -> Result<Url> {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        let mut url = self.base_url().await?;
        url.query_pairs_mut().append_pair("t", &hash.to_string());
        Ok(url)
    }
}
