use adler32::adler32;
use async_stream::try_stream;
use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{ready, FutureExt, Stream, StreamExt};
use http::HeaderMap;
use native_tls::TlsConnector;
use reqwest::{Client, ClientBuilder, Response};
use std::fmt::Debug;
use std::{pin::Pin, sync::Arc};
use std::{task::Poll, time::SystemTime};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex, RwLock,
};
use url::Url;

use crate::asynchronous::generator::StreamGenerator;
use crate::{asynchronous::transport::AsyncTransport, error::Result, Error};

#[derive(Clone, Debug)]
enum PollingEnd {
    Client(ClientPolling),
    Server(ServerPolling),
}

/// An asynchronous polling type. Makes use of the nonblocking reqwest types and
/// methods.
#[derive(Clone, Debug)]
pub struct PollingTransport {
    inner: PollingEnd,
}

impl PollingTransport {
    pub fn new(
        base_url: Url,
        tls_config: Option<TlsConnector>,
        opening_headers: Option<HeaderMap>,
    ) -> Self {
        Self {
            inner: PollingEnd::Client(ClientPolling::new(base_url, tls_config, opening_headers)),
        }
    }

    pub fn server_new(url: Url, sender: Sender<Bytes>, receiver: Receiver<Bytes>) -> Self {
        Self {
            inner: PollingEnd::Server(ServerPolling::new(url, sender, receiver)),
        }
    }
}

impl Stream for PollingTransport {
    type Item = Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match &mut self.inner {
            PollingEnd::Client(client) => client.poll_next_unpin(cx),
            PollingEnd::Server(server) => server.poll_next_unpin(cx),
        }
    }
}

#[async_trait]
impl AsyncTransport for PollingTransport {
    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        match &self.inner {
            PollingEnd::Client(client) => client.emit(data, is_binary_att).await,
            PollingEnd::Server(server) => server.emit(data, is_binary_att).await,
        }
    }

    /// Returns start of the url. ex. http://localhost:2998/engine.io/?EIO=4&transport=polling
    /// Must have EIO and transport already set.
    async fn base_url(&self) -> Result<Url> {
        match &self.inner {
            PollingEnd::Client(client) => client.base_url().await,
            PollingEnd::Server(server) => server.base_url().await,
        }
    }

    /// Used to update the base path, like when adding the sid.
    async fn set_base_url(&self, base_url: Url) -> Result<()> {
        match &self.inner {
            PollingEnd::Client(client) => client.set_base_url(base_url).await,
            PollingEnd::Server(server) => server.set_base_url(base_url).await,
        }
    }
}

#[derive(Clone)]
pub struct ClientPolling {
    client: Client,
    base_url: Arc<RwLock<Url>>,
    generator: StreamGenerator<Bytes>,
}

impl ClientPolling {
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

        ClientPolling {
            client: client.clone(),
            base_url: Arc::new(RwLock::new(url.clone())),
            generator: StreamGenerator::new(Self::stream(url, client)),
        }
    }

    fn address(mut url: Url) -> Result<Url> {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        url.query_pairs_mut().append_pair("t", &hash.to_string());
        Ok(url)
    }

    fn send_request(url: Url, client: Client) -> impl Stream<Item = Result<Response>> {
        try_stream! {
            let address = Self::address(url);

            yield client
                .get(address?)
                .send().await?
        }
    }

    fn stream(
        url: Url,
        client: Client,
    ) -> Pin<Box<dyn Stream<Item = Result<Bytes>> + 'static + Send>> {
        Box::pin(try_stream! {
            loop {
                for await elem in Self::send_request(url.clone(), client.clone()) {
                    for await bytes in elem?.bytes_stream() {
                        yield bytes?;
                    }
                }
            }
        })
    }
}

impl Stream for ClientPolling {
    type Item = Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.generator.poll_next_unpin(cx)
    }
}

#[async_trait]
impl AsyncTransport for ClientPolling {
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

impl Debug for ClientPolling {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PollingTransport")
            .field("client", &self.client)
            .field("base_url", &self.base_url)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct ServerPolling {
    sender: Arc<Mutex<Sender<Bytes>>>,
    receiver: Arc<Mutex<Receiver<Bytes>>>,
    url: Arc<Mutex<Url>>,
}

impl ServerPolling {
    fn new(url: Url, sender: Sender<Bytes>, receiver: Receiver<Bytes>) -> Self {
        // let (tx, rx) = channel(buffer_size);
        Self {
            url: Arc::new(Mutex::new(url)),
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }
}

#[async_trait]
impl AsyncTransport for ServerPolling {
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
        let sender = self.sender.lock().await;
        sender
            .send(data_to_send)
            .await
            .map_err(|e| Error::FailedToEmit(e.to_string()))?;
        Ok(())
    }

    async fn base_url(&self) -> Result<Url> {
        let url = self.url.lock().await;
        return Ok(url.clone());
    }

    async fn set_base_url(&self, base_url: Url) -> Result<()> {
        let mut url = self.url.lock().await;
        *url = base_url;
        Ok(())
    }
}

impl Stream for ServerPolling {
    type Item = Result<Bytes>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let mut lock = ready!(Box::pin(self.receiver.lock()).poll_unpin(cx));
        let recv = ready!(Box::pin(lock.recv()).poll_unpin(cx));
        drop(lock);

        match recv {
            Some(bytes) => Poll::Ready(Some(Ok(bytes))),
            None => Poll::Ready(None),
        }
    }
}

#[cfg(test)]
mod test {
    use tokio::sync::mpsc::channel;

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

    #[tokio::test]
    async fn test_server_polling_transport() -> Result<()> {
        let url = Url::parse("http://127.0.0.1/?transport=polling").unwrap();
        let (send_tx, mut send_rx) = channel(100);
        let (recv_tx, recv_rx) = channel(100);
        let mut transport = ServerPolling::new(url, send_tx, recv_rx);

        let data = Bytes::from_static(b"1Hello\x1e1HelloWorld");

        recv_tx.send(data.clone()).await.unwrap();

        let msg = transport.next().await;
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert!(msg.is_ok());
        let msg = msg.unwrap();

        assert_eq!(msg, data);

        transport.emit(data.clone(), false).await?;
        let msg = send_rx.recv().await;
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg, data);

        Ok(())
    }
}
