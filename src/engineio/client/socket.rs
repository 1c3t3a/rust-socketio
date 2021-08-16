use super::super::socket::Socket as InnerSocket;
use crate::engineio::transport::Transport;

use super::super::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};
use crate::engineio::packet::{HandshakePacket, Packet, PacketId, Payload};
use crate::error::{Error, Result};
use bytes::Bytes;
use native_tls::TlsConnector;
use reqwest::header::HeaderMap;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::fmt::Debug;
use url::Url;
use websocket::header::Headers;

#[derive(Clone, Debug)]
pub struct Socket {
    socket: InnerSocket,
}

#[derive(Clone, Debug)]
pub struct SocketBuilder {
    url: Url,
    tls_config: Option<TlsConnector>,
    headers: Option<HeaderMap>,
    handshake: Option<HandshakePacket>,
}

impl SocketBuilder {
    pub fn new(url: Url) -> Self {
        SocketBuilder {
            url,
            headers: None,
            tls_config: None,
            handshake: None,
        }
    }

    pub fn tls_config(mut self, tls_config: TlsConnector) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    pub fn headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers);
        self
    }

    fn handshake(&mut self) -> Result<()> {
        // No need to handshake twice
        if self.handshake.is_some() {
            return Ok(());
        }

        let mut url = self.url.clone();
        url.query_pairs_mut().append_pair("EIO", "4");

        // No path add engine.io
        if url.path() == "/" {
            url.set_path("/engine.io/");
        }

        // Start with polling transport
        let transport =
            PollingTransport::new(url.clone(), self.tls_config.clone(), self.headers.clone());

        let handshake: HandshakePacket = Packet::try_from(transport.poll()?)?.try_into()?;

        // update the base_url with the new sid
        url.query_pairs_mut().append_pair("sid", &handshake.sid[..]);

        self.handshake = Some(handshake);

        self.url = url;

        Ok(())
    }

    /// Build websocket if allowed, if not fall back to polling
    pub fn build(mut self) -> Result<Socket> {
        if self.websocket_upgrade()? {
            match self.url.scheme() {
                "http" => self.build_websocket(),
                "https" => self.build_websocket_secure(),
                _ => self.build_polling(),
            }
        } else {
            self.build_polling()
        }
    }

    /// Build socket with polling transport
    pub fn build_polling(mut self) -> Result<Socket> {
        self.handshake()?;

        // Make a polling transport with new sid
        let transport = PollingTransport::new(self.url, self.tls_config, self.headers);

        // SAFETY: handshake function called previously.
        Ok(Socket {
            socket: InnerSocket::new(transport.into(), self.handshake.unwrap()),
        })
    }

    /// Build socket with insecure websocket transport
    pub fn build_websocket(mut self) -> Result<Socket> {
        self.handshake()?;

        // SAFETY: Already a Url
        let url = websocket::client::Url::parse(&self.url.to_string())?;

        if self.websocket_upgrade()? {
            if url.scheme() == "http" {
                let transport = WebsocketTransport::new(url, self.get_ws_headers()?);
                transport.upgrade()?;
                // SAFETY: handshake function called previously.
                Ok(Socket {
                    socket: InnerSocket::new(transport.into(), self.handshake.unwrap()),
                })
            } else {
                Err(Error::InvalidUrlScheme(url.scheme().to_string()))
            }
        } else {
            Err(Error::IllegalWebsocketUpgrade())
        }
    }

    /// Build socket with secure websocket transport
    pub fn build_websocket_secure(mut self) -> Result<Socket> {
        self.handshake()?;

        // SAFETY: Already a Url
        let url = websocket::client::Url::parse(&self.url.to_string())?;

        if self.websocket_upgrade()? {
            if url.scheme() == "https" {
                let transport = WebsocketSecureTransport::new(
                    url,
                    self.tls_config.clone(),
                    self.get_ws_headers()?,
                );
                transport.upgrade()?;
                // SAFETY: handshake function called previously.
                Ok(Socket {
                    socket: InnerSocket::new(transport.into(), self.handshake.unwrap()),
                })
            } else {
                Err(Error::InvalidUrlScheme(url.scheme().to_string()))
            }
        } else {
            Err(Error::IllegalWebsocketUpgrade())
        }
    }

    /// Build websocket if allowed, if not allowed or errored fall back to polling.
    /// WARNING: websocket errors suppressed, no indication of websocket success or failure.
    pub fn build_with_fallback(self) -> Result<Socket> {
        let result = self.clone().build();
        if result.is_err() {
            self.build_polling()
        } else {
            result
        }
    }

    /// Checks the handshake to see if websocket upgrades are allowed
    fn websocket_upgrade(&mut self) -> Result<bool> {
        // check if we could upgrade to websockets
        self.handshake()?;
        // SAFETY: handshake set by above function.
        Ok(self
            .handshake
            .as_ref()
            .unwrap()
            .upgrades
            .iter()
            .any(|upgrade| upgrade.to_lowercase() == *"websocket"))
    }

    /// Converts Reqwest headers to Websocket headers
    fn get_ws_headers(&self) -> Result<Option<Headers>> {
        let mut headers = Headers::new();
        if self.headers.is_some() {
            let opening_headers = self.headers.clone();
            for (key, val) in opening_headers.unwrap() {
                headers.append_raw(key.unwrap().to_string(), val.as_bytes().to_owned());
            }
            Ok(Some(headers))
        } else {
            Ok(None)
        }
    }
}

impl Socket {
    /// Registers the `on_open` callback.
    pub fn on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        self.socket.on_open(function)
    }

    /// Registers the `on_error` callback.
    pub fn on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        self.socket.on_error(function)
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        self.socket.on_packet(function)
    }

    /// Registers the `on_data` callback.
    pub fn on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send,
    {
        self.socket.on_data(function)
    }

    /// Registers the `on_close` callback.
    pub fn on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        self.socket.on_close(function)
    }

    pub fn close(&mut self) -> Result<()> {
        self.socket.close()
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub fn connect(&mut self) -> Result<()> {
        self.socket.connect()
    }

    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    pub fn emit(&self, packet: Packet, is_binary_att: bool) -> Result<()> {
        self.socket.emit(packet, is_binary_att)
    }

    /// Polls for next payload
    pub(crate) fn poll(&self) -> Result<Option<Payload>> {
        let payload = self.socket.poll()?;

        if payload.is_none() {
            return Ok(None);
        }

        if let Some(payload) = payload.clone() {
            for packet in payload {
                // check for the appropriate action or callback
                self.socket.handle_packet(packet.clone())?;
                match packet.packet_id {
                    PacketId::MessageBase64 => {
                        self.socket.handle_data(packet.data)?;
                    }
                    PacketId::Message => {
                        self.socket.handle_data(packet.data)?;
                    }

                    PacketId::Close => {
                        self.socket.handle_close()?;
                        // set current state to not connected and stop polling
                        self.socket.close()?;
                    }
                    PacketId::Open => {
                        unreachable!("Won't happen as we open the connection beforehand");
                    }
                    PacketId::Upgrade => {
                        // this is already checked during the handshake, so just do nothing here
                    }
                    PacketId::Ping => {
                        self.socket.pinged()?;
                        self.emit(Packet::new(PacketId::Pong, Bytes::new()), false)?;
                    }
                    PacketId::Pong => {
                        // this will never happen as the pong packet is
                        // only sent by the client
                        unreachable!();
                    }
                    PacketId::Noop => (),
                }
            }
        }
        Ok(payload)
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        self.socket.is_connected()
    }
}

#[cfg(test)]
mod test {

    use std::thread::sleep;
    use std::time::Duration;

    use crate::engineio::packet::PacketId;

    use super::*;
    use std::thread;

    #[test]
    fn test_basic_connection() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let mut socket = SocketBuilder::new(url).build()?;

        socket.on_open(|_| {
            println!("Open event!");
        })?;

        socket.on_packet(|packet| {
            println!("Received packet: {:?}", packet);
        })?;

        socket.on_data(|data| {
            println!("Received packet: {:?}", std::str::from_utf8(&data));
        })?;

        socket.connect()?;

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"Hello World")),
            false,
        )?;

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"Hello World2")),
            false,
        )?;

        socket.emit(Packet::new(PacketId::Pong, Bytes::new()), false)?;

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"Hello World3")),
            false,
        )?;

        let mut sut = socket.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let result = sut.close();
            if result.is_ok() {
                break;
            } else if let Err(error) = result {
                println!("Closing thread errored! Trying again... {}", error);
            }
        });

        loop {
            let result = socket.poll()?;
            if result.is_none() {
                break;
            }
            // Ping/pongs/callbacks are called by poll
        }
        Ok(())
    }

    #[test]
    fn test_illegal_actions() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let mut sut = SocketBuilder::new(url.clone()).build()?;

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .is_err());
        assert!(sut
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"")),
                true
            )
            .is_err());

        sut.connect()?;

        assert!(sut.on_open(|_| {}).is_err());
        assert!(sut.on_close(|_| {}).is_err());
        assert!(sut.on_packet(|_| {}).is_err());
        assert!(sut.on_data(|_| {}).is_err());
        assert!(sut.on_error(|_| {}).is_err());

        let mut socket = sut.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let result = socket.close();
            if result.is_ok() {
                break;
            } else if let Err(error) = result {
                println!("Closing thread errored! Trying again... {}", error);
            }
        });

        loop {
            let result = sut.poll()?;
            if result.is_none() {
                break;
            }
            // Ping/pongs/callbacks are called by poll
        }

        let sut = SocketBuilder::new(url).build()?;
        assert!(sut.poll().is_err());

        Ok(())
    }
    use reqwest::header::HOST;

    use crate::engineio::packet::Packet;

    #[test]
    fn test_connection_polling() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let mut socket = SocketBuilder::new(url).build_polling()?;

        socket.on_data(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        })?;

        socket.connect().unwrap();

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
            false,
        )?;

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"Hi")),
            true,
        )?;

        let mut sut = socket.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let result = sut.close();
            if result.is_ok() {
                break;
            } else if let Err(error) = result {
                println!("Closing thread errored! Trying again... {}", error);
            }
        });

        loop {
            let result = socket.poll()?;
            if result.is_none() {
                break;
            }
            // Ping/pongs/callbacks are called by poll
            sleep(Duration::from_millis(100));
        }

        Ok(())
    }

    #[test]
    fn test_connection_secure_ws_http() -> Result<()> {
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        let url = crate::engineio::test::engine_io_server_secure()?;

        let mut headers = HeaderMap::new();
        headers.insert(HOST, host.parse().unwrap());
        let mut builder = SocketBuilder::new(url);

        builder = builder.tls_config(crate::test::tls_connector()?);
        builder = builder.headers(headers);
        let mut socket = builder.build_websocket_secure()?;

        socket.on_data(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        })?;

        socket.connect().unwrap();

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
            false,
        )?;

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"Hi")),
            true,
        )?;

        let mut sut = socket.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let result = sut.close();
            if result.is_ok() {
                break;
            } else if let Err(error) = result {
                println!("Closing thread errored! Trying again... {}", error);
            }
        });

        loop {
            let result = socket.poll()?;
            if result.is_none() {
                break;
            }
            // Ping/pongs/callbacks are called by poll
        }
        Ok(())
    }

    #[test]
    fn test_connection_ws_http() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;

        let builder = SocketBuilder::new(url);
        let mut socket = builder.build_websocket()?;

        socket.on_data(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        })?;

        socket.connect().unwrap();

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
            false,
        )?;

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"Hi")),
            true,
        )?;

        let mut sut = socket.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(3));
            let result = sut.close();
            if result.is_ok() {
                break;
            } else if let Err(error) = result {
                println!("Closing thread errored! Trying again... {}", error);
            }
        });

        loop {
            let result = socket.poll()?;
            if result.is_none() {
                break;
            }

            // Ping/pongs/callbacks are called by poll
        }
        Ok(())
    }

    #[test]
    fn test_open_invariants() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let illegal_url = "this is illegal";

        assert!(Url::parse(&illegal_url).is_err());

        let invalid_protocol = "file:///tmp/foo";
        assert!(SocketBuilder::new(Url::parse(&invalid_protocol).unwrap())
            .build()
            .is_err());

        let sut = SocketBuilder::new(url.clone()).build()?;
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .expect_err("error");
        assert!(matches!(Error::IllegalActionBeforeOpen(), _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::new();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host.parse().unwrap());

        let _ = SocketBuilder::new(url.clone())
            .tls_config(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap(),
            )
            .build()?;
        let _ = SocketBuilder::new(url).headers(headers).build()?;
        Ok(())
    }
}
