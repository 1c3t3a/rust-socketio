use crate::engineio::transport::{Transport, TransportType};

use super::transports::{PollingTransport, WebsocketSecureTransport, WebsocketTransport};
use crate::engineio::packet::{HandshakePacket, Packet, PacketId, Payload};
use crate::error::{Error, Result};
use bytes::Bytes;
use native_tls::TlsConnector;
use reqwest::header::HeaderMap;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::thread::sleep;
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use url::Url;
use websocket::header::Headers;

/// Type of a `Callback` function. (Normal closures can be passed in here).
type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I) + 'static + Sync + Send>>>>;

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone)]
pub struct Socket {
    transport: Arc<TransportType>,
    //TODO: Store these in a map
    pub on_error: Callback<String>,
    pub on_open: Callback<()>,
    pub on_close: Callback<()>,
    pub on_data: Callback<Bytes>,
    pub on_packet: Callback<Packet>,
    pub connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    connection_data: Arc<HandshakePacket>,
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
        Ok(Socket::new(transport.into(), self.handshake.unwrap()))
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
                Ok(Socket::new(transport.into(), self.handshake.unwrap()))
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
                Ok(Socket::new(transport.into(), self.handshake.unwrap()))
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
    pub fn new(transport: TransportType, handshake: HandshakePacket) -> Self {
        Socket {
            on_error: Arc::new(RwLock::new(None)),
            on_open: Arc::new(RwLock::new(None)),
            on_close: Arc::new(RwLock::new(None)),
            on_data: Arc::new(RwLock::new(None)),
            on_packet: Arc::new(RwLock::new(None)),
            transport: Arc::new(transport),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
        }
    }

    /// Registers the `on_open` callback.
    pub fn on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_open = self.on_open.write()?;
        *on_open = Some(Box::new(function));
        drop(on_open);
        Ok(())
    }

    /// Registers the `on_error` callback.
    pub fn on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_error = self.on_error.write()?;
        *on_error = Some(Box::new(function));
        drop(on_error);
        Ok(())
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_packet = self.on_packet.write()?;
        *on_packet = Some(Box::new(function));
        drop(on_packet);
        Ok(())
    }

    /// Registers the `on_data` callback.
    pub fn on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_data = self.on_data.write()?;
        *on_data = Some(Box::new(function));
        drop(on_data);
        Ok(())
    }

    /// Registers the `on_close` callback.
    pub fn on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_close = self.on_close.write()?;
        *on_close = Some(Box::new(function));
        drop(on_close);
        Ok(())
    }

    pub fn close(&mut self) -> Result<()> {
        self.emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)?;
        self.connected.store(false, Ordering::Release);
        Ok(())
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub fn connect(&mut self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.read()?.as_ref() {
            spawn_scoped!(on_open(()));
        }

        // set the last ping to now and set the connected state
        *self.last_ping.lock()? = Instant::now();

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new()), false)?;

        Ok(())
    }

    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    //TODO: Add a new PacketId to send raw binary so is_binary_att can be removed.
    pub fn emit(&self, packet: Packet, is_binary_att: bool) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }

        // send a post request with the encoded payload as body
        // if this is a binary attachment, then send the raw bytes
        let data: Bytes = if is_binary_att {
            packet.data
        } else {
            packet.into()
        };

        if let Err(error) = self.transport.as_transport().emit(data, is_binary_att) {
            self.call_error_callback(error.to_string())?;
            return Err(error);
        }

        Ok(())
    }

    /// Polls for next payload
    pub(crate) fn poll(&self) -> Result<Option<Payload>> {
        if self.connected.load(Ordering::Acquire) {
            let data = self.transport.as_transport().poll()?;

            if data.is_empty() {
                return Ok(None);
            }

            Ok(Some(Payload::try_from(data)?))
        } else {
            Err(Error::IllegalActionAfterClose())
        }
    }

    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    pub(crate) fn poll_cycle(&self) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }

        // as we don't have a mut self, the last_ping needs to be safed for later
        let mut last_ping = *self.last_ping.lock()?;
        // the time after we assume the server to be timed out
        let server_timeout = Duration::from_millis(
            self.connection_data.ping_timeout + self.connection_data.ping_interval,
        );

        while self.connected.load(Ordering::Acquire) {
            let packets = self.poll();

            // Double check that we have not disconnected since last loop.
            if !self.connected.load(Ordering::Acquire) {
                break;
            }

            // Only care about errors if the connection is still open.
            let packets = packets?;

            if packets.is_none() {
                break;
            }

            // SAFETY: packets checked to be none before
            for packet in packets.unwrap() {
                // check for the appropriate action or callback
                if let Some(on_packet) = self.on_packet.read()?.as_ref() {
                    spawn_scoped!(on_packet(packet.clone()));
                }
                match packet.packet_id {
                    PacketId::MessageBase64 => {
                        if let Some(on_data) = self.on_data.read()?.as_ref() {
                            spawn_scoped!(on_data(packet.data));
                        }
                    }
                    PacketId::Message => {
                        if let Some(on_data) = self.on_data.read()?.as_ref() {
                            spawn_scoped!(on_data(packet.data));
                        }
                    }

                    PacketId::Close => {
                        if let Some(on_close) = self.on_close.read()?.as_ref() {
                            spawn_scoped!(on_close(()));
                        }
                        // set current state to not connected and stop polling
                        self.connected.store(false, Ordering::Release);
                    }
                    PacketId::Open => {
                        unreachable!("Won't happen as we open the connection beforehand");
                    }
                    PacketId::Upgrade => {
                        // this is already checked during the handshake, so just do nothing here
                    }
                    PacketId::Ping => {
                        last_ping = Instant::now();
                        let result = self.emit(Packet::new(PacketId::Pong, Bytes::new()), false);
                        // Only care about errors if the socket is still connected
                        if self.connected.load(Ordering::Acquire) {
                            result?;
                        }
                    }
                    PacketId::Pong => {
                        // this will never happen as the pong packet is
                        // only sent by the client
                        unreachable!();
                    }
                    PacketId::Noop => (),
                }
            }

            // TODO: respect timeout properly.
            // This check only happens after a valid response from the server
            if server_timeout < last_ping.elapsed() {
                // the server is unreachable
                // set current state to not connected and stop polling
                self.connected.store(false, Ordering::Release);
            }

            // Sleep for a little while in between loops so any emits can send.
            sleep(Duration::from_secs(1));
        }
        Ok(())
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) -> Result<()> {
        let function = self.on_error.read()?;
        if let Some(function) = function.as_ref() {
            spawn_scoped!(function(text));
        }
        drop(function);

        Ok(())
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }
}

impl Debug for Socket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocket(transport: {:?}, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_ping: {:?}, last_pong: {:?}, connection_data: {:?})",
            self.transport,
            if self.on_error.read().unwrap().is_some() {
                "Fn(String)"
            } else {
                "None"
            },
            if self.on_open.read().unwrap().is_some() {
                "Fn(())"
            } else {
                "None"
            },
            if self.on_close.read().unwrap().is_some() {
                "Fn(())"
            } else {
                "None"
            },
            if self.on_packet.read().unwrap().is_some() {
                "Fn(Packet)"
            } else {
                "None"
            },
            if self.on_data.read().unwrap().is_some() {
                "Fn(Bytes)"
            } else {
                "None"
            },
            self.connected,
            self.last_ping,
            self.last_pong,
            self.connection_data,
        ))
    }
}

#[cfg(test)]
mod test {

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

        socket.poll_cycle()?;
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

        sut.poll_cycle()?;

        let sut = SocketBuilder::new(url).build()?;
        assert!(sut.poll_cycle().is_err());

        Ok(())
    }
    use reqwest::header::HOST;

    use crate::engineio::packet::Packet;

    #[test]
    fn test_connection_polling() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let mut socket = SocketBuilder::new(url).build_polling()?;

        socket.connect().unwrap();

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
            false,
        )?;

        socket.on_data = Arc::new(RwLock::new(Some(Box::new(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        }))));

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

        socket.poll_cycle()?;

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

        socket.connect().unwrap();

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
            false,
        )?;

        socket.on_data = Arc::new(RwLock::new(Some(Box::new(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        }))));

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

        socket.poll_cycle()?;
        Ok(())
    }

    #[test]
    fn test_connection_ws_http() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;

        let builder = SocketBuilder::new(url);
        let mut socket = builder.build_websocket()?;

        socket.connect().unwrap();

        socket.emit(
            Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
            false,
        )?;

        socket.on_data = Arc::new(RwLock::new(Some(Box::new(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        }))));

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

        socket.poll_cycle()?;
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
