use async_stream::try_stream;
use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::Stream;
use http::HeaderMap;
use native_tls::TlsConnector;
use reqwest::{Client, ClientBuilder, Response};
use std::{pin::Pin, sync::Arc};
use tokio::sync::RwLock;
use url::Url;

use crate::{asynchronous::transport::AsyncTransport, error::Result, Error};

/// An asynchronous polling type. Makes use of the nonblocking reqwest types and
/// methods.
#[derive(Clone, Debug)]
pub struct PollingTransport {
    client: Client,
    base_url: Arc<RwLock<Url>>,
}

impl PollingTransport {
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

        PollingTransport {
            client,
            base_url: Arc::new(RwLock::new(url)),
        }
    }

    fn send_request(&self) -> impl Stream<Item = Result<Response>> + '_ {
        try_stream! {
            let address = self.address().await;

            yield self
                .client
                .get(address?)
                .send().await?
        }
    }
}

#[async_trait]
impl AsyncTransport for PollingTransport {
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

    fn stream(&self) -> Pin<Box<dyn Stream<Item = Result<Bytes>> + '_>> {
        Box::pin(try_stream! {
            // always send the long polling request until the transport is shut down

            // TODO: Investigate a way to stop this. From my understanding this generator
            // lives on the heap (as it's boxed) and always sends long polling
            // requests. In the current version of the crate it is possible to create a stream
            // using an `&self` of client, which would mean that we potentially end up with multiple
            // of these generators on the heap. That is not what we want.
            // A potential solution would be to wrap the client (also helpful for websockets) and make
            // sure to call the "poll" request only on the same object over and over again. In that
            // case the underlying object would have information about whether the connection is shut down
            // or not.
            // This stream never returns `None` and hence never indicates an end.
            loop {
                for await elem in self.send_request() {
                    for await bytes in elem?.bytes_stream() {
                        yield bytes?;
                    }
                }

            }
        })
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

#[cfg(test)]
mod test {
    use crate::asynchronous::transport::AsyncTransport;

    use super::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn polling_transport_base_url() -> Result<()> {
        let url = crate::test::engine_io_server()?.to_string();
        let transport = PollingTransport::new(Url::from_str(&url[..]).unwrap(), None, None);
        assert_eq!(
            transport.base_url().await?.to_string(),
            url.clone() + "?transport=polling"
        );
        transport
            .set_base_url(Url::parse("https://127.0.0.1")?)
            .await?;
        assert_eq!(
            transport.base_url().await?.to_string(),
            "https://127.0.0.1/?transport=polling"
        );
        assert_ne!(transport.base_url().await?.to_string(), url);

        transport
            .set_base_url(Url::parse("http://127.0.0.1/?transport=polling")?)
            .await?;
        assert_eq!(
            transport.base_url().await?.to_string(),
            "http://127.0.0.1/?transport=polling"
        );
        assert_ne!(transport.base_url().await?.to_string(), url);
        Ok(())
    }
}
