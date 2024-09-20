use crate::{
    asynchronous::{
        async_socket::Socket as InnerSocket,
        async_transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport},
        callback::OptionalCallback,
        transport::AsyncTransport,
    },
    error::Result,
    header::HeaderMap,
    packet::HandshakePacket,
    Error, Packet, TlsConfig, ENGINE_IO_VERSION,
};
use bytes::Bytes;
use futures_util::{future::BoxFuture, StreamExt};
use url::Url;

use super::Client;

#[derive(Clone, Debug)]
pub struct ClientBuilder {
    url: Url,
    tls_config: Option<TlsConfig>,
    headers: Option<HeaderMap>,
    handshake: Option<HandshakePacket>,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_packet: OptionalCallback<Packet>,
}

impl ClientBuilder {
    pub fn new(url: Url) -> Self {
        let mut url = url;
        url.query_pairs_mut()
            .append_pair("EIO", &ENGINE_IO_VERSION.to_string());

        // No path add engine.io
        if url.path() == "/" {
            url.set_path("/engine.io/");
        }
        ClientBuilder {
            url,
            headers: None,
            tls_config: None,
            handshake: None,
            on_close: OptionalCallback::default(),
            on_data: OptionalCallback::default(),
            on_error: OptionalCallback::default(),
            on_open: OptionalCallback::default(),
            on_packet: OptionalCallback::default(),
        }
    }

    /// Specify transport's tls config
    pub fn tls_config(mut self, tls_config: TlsConfig) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Specify transport's HTTP headers
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Registers the `on_close` callback.
    #[cfg(feature = "async-callbacks")]
    pub fn on_close<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(()) -> BoxFuture<'static, ()>,
    {
        self.on_close = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_data` callback.
    #[cfg(feature = "async-callbacks")]
    pub fn on_data<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(Bytes) -> BoxFuture<'static, ()>,
    {
        self.on_data = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_error` callback.
    #[cfg(feature = "async-callbacks")]
    pub fn on_error<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(String) -> BoxFuture<'static, ()>,
    {
        self.on_error = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_open` callback.
    #[cfg(feature = "async-callbacks")]
    pub fn on_open<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(()) -> BoxFuture<'static, ()>,
    {
        self.on_open = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_packet` callback.
    #[cfg(feature = "async-callbacks")]
    pub fn on_packet<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(Packet) -> BoxFuture<'static, ()>,
    {
        self.on_packet = OptionalCallback::new(callback);
        self
    }

    /// Performs the handshake
    async fn handshake_with_transport<T: AsyncTransport + Unpin>(
        &mut self,
        transport: &mut T,
    ) -> Result<()> {
        // No need to handshake twice
        if self.handshake.is_some() {
            return Ok(());
        }

        let mut url = self.url.clone();

        let handshake: HandshakePacket =
            Packet::try_from(transport.next().await.ok_or(Error::IncompletePacket())??)?
                .try_into()?;

        // update the base_url with the new sid
        url.query_pairs_mut().append_pair("sid", &handshake.sid[..]);

        self.handshake = Some(handshake);

        self.url = url;

        Ok(())
    }

    async fn handshake(&mut self) -> Result<()> {
        if self.handshake.is_some() {
            return Ok(());
        }

        let headers = if let Some(map) = self.headers.clone() {
            Some(map.try_into()?)
        } else {
            None
        };

        // Start with polling transport
        let mut transport =
            PollingTransport::new(self.url.clone(), self.tls_config.clone(), headers);

        self.handshake_with_transport(&mut transport).await
    }

    /// Build websocket if allowed, if not fall back to polling
    pub async fn build(mut self) -> Result<Client> {
        self.handshake().await?;

        if self.websocket_upgrade()? {
            self.build_websocket_with_upgrade().await
        } else {
            self.build_polling().await
        }
    }

    /// Build socket with polling transport
    pub async fn build_polling(mut self) -> Result<Client> {
        self.handshake().await?;

        // Make a polling transport with new sid
        let transport = PollingTransport::new(
            self.url,
            self.tls_config,
            self.headers.map(|v| v.try_into().unwrap()),
        );

        // SAFETY: handshake function called previously.
        Ok(Client::new(InnerSocket::new(
            transport.into(),
            self.handshake.unwrap(),
            self.on_close,
            self.on_data,
            self.on_error,
            self.on_open,
            self.on_packet,
        )))
    }

    /// Build socket with a polling transport then upgrade to websocket transport
    pub async fn build_websocket_with_upgrade(mut self) -> Result<Client> {
        self.handshake().await?;

        if self.websocket_upgrade()? {
            self.build_websocket().await
        } else {
            Err(Error::IllegalWebsocketUpgrade())
        }
    }

    /// Build socket with only a websocket transport
    pub async fn build_websocket(mut self) -> Result<Client> {
        let headers = if let Some(map) = self.headers.clone() {
            Some(map.try_into()?)
        } else {
            None
        };

        match self.url.scheme() {
            "http" | "ws" => {
                let mut transport = WebsocketTransport::new(self.url.clone(), headers).await?;

                if self.handshake.is_some() {
                    transport.upgrade().await?;
                } else {
                    self.handshake_with_transport(&mut transport).await?;
                }
                // NOTE: Although self.url contains the sid, it does not propagate to the transport
                // SAFETY: handshake function called previously.
                Ok(Client::new(InnerSocket::new(
                    transport.into(),
                    self.handshake.unwrap(),
                    self.on_close,
                    self.on_data,
                    self.on_error,
                    self.on_open,
                    self.on_packet,
                )))
            }
            "https" | "wss" => {
                let mut transport = WebsocketSecureTransport::new(
                    self.url.clone(),
                    self.tls_config.clone(),
                    headers,
                )
                .await?;

                if self.handshake.is_some() {
                    transport.upgrade().await?;
                } else {
                    self.handshake_with_transport(&mut transport).await?;
                }
                // NOTE: Although self.url contains the sid, it does not propagate to the transport
                // SAFETY: handshake function called previously.
                Ok(Client::new(InnerSocket::new(
                    transport.into(),
                    self.handshake.unwrap(),
                    self.on_close,
                    self.on_data,
                    self.on_error,
                    self.on_open,
                    self.on_packet,
                )))
            }
            _ => Err(Error::InvalidUrlScheme(self.url.scheme().to_string())),
        }
    }

    /// Build websocket if allowed, if not allowed or errored fall back to polling.
    /// WARNING: websocket errors suppressed, no indication of websocket success or failure.
    pub async fn build_with_fallback(self) -> Result<Client> {
        let result = self.clone().build().await;
        if result.is_err() {
            self.build_polling().await
        } else {
            result
        }
    }

    /// Checks the handshake to see if websocket upgrades are allowed
    fn websocket_upgrade(&mut self) -> Result<bool> {
        if self.handshake.is_none() {
            return Ok(false);
        }

        Ok(self
            .handshake
            .as_ref()
            .unwrap()
            .upgrades
            .iter()
            .any(|upgrade| upgrade.to_lowercase() == *"websocket"))
    }
}
