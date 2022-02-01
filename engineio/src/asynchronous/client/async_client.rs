use crate::{
    asynchronous::{
        async_socket::Socket as InnerSocket,
        async_transports::{AsyncPollingTransport, WebsocketSecureTransport, WebsocketTransport},
        callback::OptionalCallback,
        transport::AsyncTransport,
    },
    error::Result,
    header::HeaderMap,
    packet::HandshakePacket,
    Error, Packet, PacketId, ENGINE_IO_VERSION,
};
use bytes::Bytes;
use native_tls::TlsConnector;
use std::{future::Future, pin::Pin};
use url::Url;

#[derive(Clone, Debug)]
pub struct Client {
    socket: InnerSocket,
}

#[derive(Clone, Debug)]
pub struct ClientBuilder {
    url: Url,
    tls_config: Option<TlsConnector>,
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
    pub fn tls_config(mut self, tls_config: TlsConnector) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Specify transport's HTTP headers
    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Registers the `on_close` callback.
    pub fn on_close<T>(mut self, callback: T) -> Self
    where
        T: Fn(()) -> Pin<Box<dyn Future<Output = ()>>> + 'static,
    {
        self.on_close = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_data` callback.
    pub fn on_data<T>(mut self, callback: T) -> Self
    where
        T: Fn(Bytes) -> Pin<Box<dyn Future<Output = ()>>> + 'static,
    {
        self.on_data = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_error` callback.
    pub fn on_error<T>(mut self, callback: T) -> Self
    where
        T: Fn(String) -> Pin<Box<dyn Future<Output = ()>>> + 'static,
    {
        self.on_error = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_open` callback.
    pub fn on_open<T>(mut self, callback: T) -> Self
    where
        T: Fn(()) -> Pin<Box<dyn Future<Output = ()>>> + 'static,
    {
        self.on_open = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<T>(mut self, callback: T) -> Self
    where
        T: Fn(Packet) -> Pin<Box<dyn Future<Output = ()>>> + 'static,
    {
        self.on_packet = OptionalCallback::new(callback);
        self
    }

    /// Performs the handshake
    async fn handshake_with_transport<T: AsyncTransport>(&mut self, transport: &T) -> Result<()> {
        // No need to handshake twice
        if self.handshake.is_some() {
            return Ok(());
        }

        let mut url = self.url.clone();

        let handshake: HandshakePacket = Packet::try_from(transport.poll().await?)?.try_into()?;

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
        let transport =
            AsyncPollingTransport::new(self.url.clone(), self.tls_config.clone(), headers);

        self.handshake_with_transport(&transport).await
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
        let transport = AsyncPollingTransport::new(
            self.url,
            self.tls_config,
            self.headers.map(|v| v.try_into().unwrap()),
        );

        // SAFETY: handshake function called previously.
        Ok(Client {
            socket: InnerSocket::new(
                transport.into(),
                self.handshake.unwrap(),
                self.on_close,
                self.on_data,
                self.on_error,
                self.on_open,
                self.on_packet,
            ),
        })
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
        // SAFETY: Already a Url
        let url = url::Url::parse(&self.url.to_string())?;
        let headers = if let Some(map) = self.headers.clone() {
            Some(map.try_into()?)
        } else {
            None
        };

        match url.scheme() {
            "http" | "ws" => {
                let transport = WebsocketTransport::new(url, headers).await?;

                if self.handshake.is_some() {
                    transport.upgrade().await?;
                } else {
                    self.handshake_with_transport(&transport).await?;
                }
                // NOTE: Although self.url contains the sid, it does not propagate to the transport
                // SAFETY: handshake function called previously.
                Ok(Client {
                    socket: InnerSocket::new(
                        transport.into(),
                        self.handshake.unwrap(),
                        self.on_close,
                        self.on_data,
                        self.on_error,
                        self.on_open,
                        self.on_packet,
                    ),
                })
            }
            "https" | "wss" => {
                let transport =
                    WebsocketSecureTransport::new(url, self.tls_config.clone(), headers).await?;

                if self.handshake.is_some() {
                    transport.upgrade().await?;
                } else {
                    self.handshake_with_transport(&transport).await?;
                }
                // NOTE: Although self.url contains the sid, it does not propagate to the transport
                // SAFETY: handshake function called previously.
                Ok(Client {
                    socket: InnerSocket::new(
                        transport.into(),
                        self.handshake.unwrap(),
                        self.on_close,
                        self.on_data,
                        self.on_error,
                        self.on_open,
                        self.on_packet,
                    ),
                })
            }
            _ => Err(Error::InvalidUrlScheme(url.scheme().to_string())),
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
        // SAFETY: handshake set by above function.
        Ok(self
            .handshake
            .as_ref()
            .unwrap()
            .upgrades
            .iter()
            .any(|upgrade| upgrade.to_lowercase() == *"websocket"))
    }
}

impl Client {
    pub async fn close(&self) -> Result<()> {
        self.socket.disconnect().await
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub async fn connect(&self) -> Result<()> {
        self.socket.connect().await
    }

    /// Disconnects the connection.
    pub async fn disconnect(&self) -> Result<()> {
        self.socket.disconnect().await
    }

    /// Sends a packet to the server.
    pub async fn emit(&self, packet: Packet) -> Result<()> {
        self.socket.emit(packet).await
    }

    /// Polls for next payload
    #[doc(hidden)]
    pub async fn poll(&self) -> Result<Option<Packet>> {
        let packet = self.socket.poll().await?;
        if let Some(packet) = packet {
            // check for the appropriate action or callback
            self.socket.handle_packet(packet.clone()).await;
            match packet.packet_id {
                PacketId::MessageBinary => {
                    self.socket.handle_data(packet.data.clone()).await;
                }
                PacketId::Message => {
                    self.socket.handle_data(packet.data.clone()).await;
                }
                PacketId::Close => {
                    self.socket.handle_close().await;
                }
                PacketId::Open => {
                    unreachable!("Won't happen as we open the connection beforehand");
                }
                PacketId::Upgrade => {
                    // this is already checked during the handshake, so just do nothing here
                }
                PacketId::Ping => {
                    self.socket.pinged().await?;
                    self.emit(Packet::new(PacketId::Pong, Bytes::new())).await?;
                }
                PacketId::Pong => {
                    // this will never happen as the pong packet is
                    // only sent by the client
                    unreachable!();
                }
                PacketId::Noop => (),
            }
            Ok(Some(packet))
        } else {
            Ok(None)
        }
    }

    /// Check if the underlying transport client is connected.
    pub fn is_connected(&self) -> Result<bool> {
        self.socket.is_connected()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn it_works() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let client = ClientBuilder::new(url::Url::parse("http://localhost:4201").unwrap())
                .on_error(|msg| Box::pin(async move { println!("{}", msg) }))
                .on_packet(|msg| Box::pin(async move { println!("{:?}", msg) }))
                .build()
                .await
                .unwrap();

            client.connect().await.unwrap();

            client
                .emit(Packet::new(PacketId::Message, "Foo"))
                .await
                .unwrap();
        });
    }
}
