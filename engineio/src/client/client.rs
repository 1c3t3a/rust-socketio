use super::super::socket::Socket as InnerSocket;
use crate::callback::OptionalCallback;
use crate::socket::DEFAULT_MAX_POLL_TIMEOUT;
use crate::transport::Transport;

use crate::error::{Error, Result};
use crate::header::HeaderMap;
use crate::packet::{HandshakePacket, Packet, PacketId};
use crate::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};
use crate::{TlsConfig, ENGINE_IO_VERSION};
use bytes::Bytes;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt::Debug;
use url::Url;

/// An engine.io client that allows interaction with the connected engine.io
/// server. This client provides means for connecting, disconnecting and sending
/// packets to the server.
///
/// ## Note:
/// There is no need to put this Client behind an `Arc`, as the type uses `Arc`
/// internally and provides a shared state beyond all cloned instances.
#[derive(Clone, Debug)]
pub struct Client {
    socket: InnerSocket,
}

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
    pub fn on_close<T>(mut self, callback: T) -> Self
    where
        T: Fn(()) + 'static + Sync + Send,
    {
        self.on_close = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_data` callback.
    pub fn on_data<T>(mut self, callback: T) -> Self
    where
        T: Fn(Bytes) + 'static + Sync + Send,
    {
        self.on_data = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_error` callback.
    pub fn on_error<T>(mut self, callback: T) -> Self
    where
        T: Fn(String) + 'static + Sync + Send,
    {
        self.on_error = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_open` callback.
    pub fn on_open<T>(mut self, callback: T) -> Self
    where
        T: Fn(()) + 'static + Sync + Send,
    {
        self.on_open = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<T>(mut self, callback: T) -> Self
    where
        T: Fn(Packet) + 'static + Sync + Send,
    {
        self.on_packet = OptionalCallback::new(callback);
        self
    }

    /// Performs the handshake
    fn handshake_with_transport<T: Transport>(&mut self, transport: &T) -> Result<()> {
        // No need to handshake twice
        if self.handshake.is_some() {
            return Ok(());
        }

        let mut url = self.url.clone();

        let handshake: HandshakePacket =
            Packet::try_from(transport.poll(DEFAULT_MAX_POLL_TIMEOUT)?)?.try_into()?;

        // update the base_url with the new sid
        url.query_pairs_mut().append_pair("sid", &handshake.sid[..]);

        self.handshake = Some(handshake);

        self.url = url;

        Ok(())
    }

    fn handshake(&mut self) -> Result<()> {
        if self.handshake.is_some() {
            return Ok(());
        }

        // Start with polling transport
        let transport = PollingTransport::new(
            self.url.clone(),
            self.tls_config.clone(),
            self.headers.clone().map(|v| v.try_into().unwrap()),
        );

        self.handshake_with_transport(&transport)
    }

    /// Build websocket if allowed, if not fall back to polling
    pub fn build(mut self) -> Result<Client> {
        self.handshake()?;

        if self.websocket_upgrade()? {
            self.build_websocket_with_upgrade()
        } else {
            self.build_polling()
        }
    }

    /// Build socket with polling transport
    pub fn build_polling(mut self) -> Result<Client> {
        self.handshake()?;

        // Make a polling transport with new sid
        let transport = PollingTransport::new(
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
    pub fn build_websocket_with_upgrade(mut self) -> Result<Client> {
        self.handshake()?;

        if self.websocket_upgrade()? {
            self.build_websocket()
        } else {
            Err(Error::IllegalWebsocketUpgrade())
        }
    }

    /// Build socket with only a websocket transport
    pub fn build_websocket(mut self) -> Result<Client> {
        // SAFETY: Already a Url
        let url = url::Url::parse(self.url.as_ref())?;

        let headers: Option<http::HeaderMap> = if let Some(map) = self.headers.clone() {
            Some(map.try_into()?)
        } else {
            None
        };

        match url.scheme() {
            "http" | "ws" => {
                let transport = WebsocketTransport::new(url, headers)?;
                if self.handshake.is_some() {
                    transport.upgrade()?;
                } else {
                    self.handshake_with_transport(&transport)?;
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
                    WebsocketSecureTransport::new(url, self.tls_config.clone(), headers)?;
                if self.handshake.is_some() {
                    transport.upgrade()?;
                } else {
                    self.handshake_with_transport(&transport)?;
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
    pub fn build_with_fallback(self) -> Result<Client> {
        let result = self.clone().build();
        if result.is_err() {
            self.build_polling()
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
    pub fn close(&self) -> Result<()> {
        self.socket.disconnect()
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub fn connect(&self) -> Result<()> {
        self.socket.connect()
    }

    /// Disconnects the connection.
    pub fn disconnect(&self) -> Result<()> {
        self.socket.disconnect()
    }

    /// Sends a packet to the server.
    pub fn emit(&self, packet: Packet) -> Result<()> {
        self.socket.emit(packet)
    }

    /// Polls for next payload
    #[doc(hidden)]
    pub fn poll(&self) -> Result<Option<Packet>> {
        let packet = self.socket.poll()?;
        if let Some(packet) = packet {
            // check for the appropriate action or callback
            self.socket.handle_packet(packet.clone());
            match packet.packet_id {
                PacketId::MessageBinary => {
                    self.socket.handle_data(packet.data.clone());
                }
                PacketId::Message => {
                    self.socket.handle_data(packet.data.clone());
                }
                PacketId::Close => {
                    self.socket.handle_close();
                }
                PacketId::Open => {
                    unreachable!("Won't happen as we open the connection beforehand");
                }
                PacketId::Upgrade => {
                    // this is already checked during the handshake, so just do nothing here
                }
                PacketId::Ping => {
                    self.socket.pinged()?;
                    self.emit(Packet::new(PacketId::Pong, Bytes::new()))?;
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

    pub fn iter(&self) -> Iter {
        Iter { socket: self }
    }
}

#[derive(Clone)]
pub struct Iter<'a> {
    socket: &'a Client,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<Packet>;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        match self.socket.poll() {
            Ok(Some(packet)) => Some(Ok(packet)),
            Ok(None) => None,
            Err(err) => Some(Err(err)),
        }
    }
}

#[cfg(test)]
mod test {

    use crate::{packet::PacketId, test::tls_connector};

    use super::*;

    /// The purpose of this test is to check whether the Client is properly cloneable or not.
    /// As the documentation of the engine.io client states, the object needs to maintain it's internal
    /// state when cloned and the cloned object should reflect the same state throughout the lifetime
    /// of both objects (initial and cloned).
    #[test]
    fn test_client_cloneable() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let sut = builder(url).build()?;

        let cloned = sut.clone();

        sut.connect()?;

        // when the underlying socket is connected, the
        // state should also change on the cloned one
        assert!(sut.is_connected()?);
        assert!(cloned.is_connected()?);

        // both clients should reflect the same messages.
        let mut iter = sut
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_id != PacketId::Ping);

        let mut iter_cloned = cloned
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_id != PacketId::Ping);

        assert_eq!(
            iter.next(),
            Some(Packet::new(PacketId::Message, "hello client"))
        );

        sut.emit(Packet::new(PacketId::Message, "respond"))?;

        assert_eq!(
            iter_cloned.next(),
            Some(Packet::new(PacketId::Message, "Roger Roger"))
        );

        cloned.disconnect()?;

        // when the underlying socket is disconnected, the
        // state should also change on the cloned one
        assert!(!sut.is_connected()?);
        assert!(!cloned.is_connected()?);

        Ok(())
    }

    #[test]
    fn test_illegal_actions() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let sut = builder(url.clone()).build()?;

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::new()))
            .is_err());

        sut.connect()?;

        assert!(sut.poll().is_ok());

        assert!(builder(Url::parse("fake://fake.fake").unwrap())
            .build_websocket()
            .is_err());

        Ok(())
    }
    use reqwest::header::HOST;

    use crate::packet::Packet;

    fn builder(url: Url) -> ClientBuilder {
        ClientBuilder::new(url)
            .on_open(|_| {
                println!("Open event!");
            })
            .on_packet(|packet| {
                println!("Received packet: {:?}", packet);
            })
            .on_data(|data| {
                println!("Received data: {:?}", std::str::from_utf8(&data));
            })
            .on_close(|_| {
                println!("Close event!");
            })
            .on_error(|error| {
                println!("Error {}", error);
            })
    }

    fn test_connection(socket: Client) -> Result<()> {
        let socket = socket;

        socket.connect().unwrap();

        // TODO: 0.3.X better tests

        let mut iter = socket
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_id != PacketId::Ping);

        assert_eq!(
            iter.next(),
            Some(Packet::new(PacketId::Message, "hello client"))
        );

        socket.emit(Packet::new(PacketId::Message, "respond"))?;

        assert_eq!(
            iter.next(),
            Some(Packet::new(PacketId::Message, "Roger Roger"))
        );

        socket.close()
    }

    #[test]
    fn test_connection_long() -> Result<()> {
        // Long lived socket to receive pings
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build()?;

        socket.connect()?;

        let mut iter = socket.iter();
        // hello client
        iter.next();
        // Ping
        iter.next();

        socket.disconnect()?;

        assert!(!socket.is_connected()?);

        Ok(())
    }

    #[test]
    fn test_connection_dynamic() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build()?;
        test_connection(socket)?;

        let url = crate::test::engine_io_polling_server()?;
        let socket = builder(url).build()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_fallback() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build_with_fallback()?;
        test_connection(socket)?;

        let url = crate::test::engine_io_polling_server()?;
        let socket = builder(url).build_with_fallback()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_dynamic_secure() -> Result<()> {
        let url = crate::test::engine_io_server_secure()?;
        let mut builder = builder(url);
        builder = builder.tls_config(crate::test::tls_connector()?);
        let socket = builder.build()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_polling() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build_polling()?;
        test_connection(socket)
    }

    #[test]
    fn test_connection_wss() -> Result<()> {
        let url = crate::test::engine_io_polling_server()?;
        assert!(builder(url).build_websocket_with_upgrade().is_err());

        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        let mut url = crate::test::engine_io_server_secure()?;

        let mut headers = HeaderMap::default();
        headers.insert(HOST, host);
        let mut builder = builder(url.clone());

        builder = builder.tls_config(crate::test::tls_connector()?);
        builder = builder.headers(headers.clone());
        let socket = builder.clone().build_websocket_with_upgrade()?;

        test_connection(socket)?;

        let socket = builder.build_websocket()?;

        test_connection(socket)?;

        url.set_scheme("wss").unwrap();

        let builder = self::builder(url)
            .tls_config(crate::test::tls_connector()?)
            .headers(headers);
        let socket = builder.clone().build_websocket()?;

        test_connection(socket)?;

        assert!(builder.build_websocket_with_upgrade().is_err());

        Ok(())
    }

    #[test]
    fn test_connection_ws() -> Result<()> {
        let url = crate::test::engine_io_polling_server()?;
        assert!(builder(url.clone()).build_websocket().is_err());
        assert!(builder(url).build_websocket_with_upgrade().is_err());

        let mut url = crate::test::engine_io_server()?;

        let builder = builder(url.clone());
        let socket = builder.clone().build_websocket()?;
        test_connection(socket)?;

        let socket = builder.build_websocket_with_upgrade()?;
        test_connection(socket)?;

        url.set_scheme("ws").unwrap();

        let builder = self::builder(url);
        let socket = builder.clone().build_websocket()?;

        test_connection(socket)?;

        assert!(builder.build_websocket_with_upgrade().is_err());

        Ok(())
    }

    #[test]
    fn test_open_invariants() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let illegal_url = "this is illegal";

        assert!(Url::parse(illegal_url).is_err());

        let invalid_protocol = "file:///tmp/foo";
        assert!(builder(Url::parse(invalid_protocol).unwrap())
            .build()
            .is_err());

        let sut = builder(url.clone()).build()?;
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::new()))
            .expect_err("error");
        assert!(matches!(Error::IllegalActionBeforeOpen(), _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::default();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host);

        let _ = builder(url.clone())
            .tls_config(
                tls_connector()?
            )
            .build()?;
        let _ = builder(url).headers(headers).build()?;
        Ok(())
    }
}
