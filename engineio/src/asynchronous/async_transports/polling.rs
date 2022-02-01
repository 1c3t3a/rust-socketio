use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use http::HeaderMap;
use native_tls::TlsConnector;
use reqwest::{Client, ClientBuilder};
use std::sync::Arc;
use tokio::sync::RwLock;
use url::Url;

use crate::{asynchronous::transport::AsyncTransport, error::Result, Error};

#[derive(Clone, Debug)]
pub struct AsyncPollingTransport {
    client: Client,
    base_url: Arc<RwLock<Url>>,
}

impl AsyncPollingTransport {
    pub fn new(
        base_url: Url,
        tls_config: Option<TlsConnector>,
        opening_headers: Option<HeaderMap>,
    ) -> Self {
        let client = match (tls_config, opening_headers) {
            (Some(config), Some(map)) => ClientBuilder::new()
                .use_preconfigured_tls(config)
                .default_headers(map)
                .build()
                .unwrap(),
            (Some(config), None) => ClientBuilder::new()
                .use_preconfigured_tls(config)
                .build()
                .unwrap(),
            (None, Some(map)) => ClientBuilder::new().default_headers(map).build().unwrap(),
            (None, None) => Client::new(),
        };

        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "polling");

        AsyncPollingTransport {
            client,
            base_url: Arc::new(RwLock::new(url)),
        }
    }
}

#[async_trait]
impl AsyncTransport for AsyncPollingTransport {
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let data_to_send = if is_binary_att {
            // the binary attachment gets `base64` encoded
            let mut packet_bytes = BytesMut::with_capacity(data.len() + 1);
            packet_bytes.put_u8(b'b');

            let encoded_data = base64::encode(data);
            packet_bytes.put(encoded_data.as_bytes());

            packet_bytes.freeze()
        } else {
            data
        };

        let status = self
            .client
            .post(self.address().await?)
            .body(data_to_send)
            .send()
            .await?
            .status()
            .as_u16();

        if status != 200 {
            let error = Error::IncompleteHttp(status);
            return Err(error);
        }

        Ok(())
    }

    async fn poll(&self) -> Result<Bytes> {
        Ok(self
            .client
            .get(self.address().await?)
            .send()
            .await?
            .bytes()
            .await?)
    }

    async fn base_url(&self) -> Result<Url> {
        Ok(self.base_url.read().await.clone())
    }

    async fn set_base_url(&self, base_url: Url) -> Result<()> {
        let mut url = base_url;
        if !url
            .query_pairs()
            .any(|(k, v)| k == "transport" && v == "polling")
        {
            url.query_pairs_mut().append_pair("transport", "polling");
        }
        *self.base_url.write().await = url;
        Ok(())
    }
}
