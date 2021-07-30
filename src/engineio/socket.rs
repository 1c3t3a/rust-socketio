use crate::engineio::transport::Transport;
use std::thread;

use crate::engineio::packet::{Packet, PacketId, Payload, HandshakePacket};
use super::transports::{polling::PollingTransport, websocket::WebsocketTransport, websocket_secure::WebsocketSecureTransport};
use crate::error::{Error, Result};
use adler32::adler32;
use bytes::{Bytes};
use native_tls::TlsConnector;
use reqwest::{
    header::HeaderMap,
};
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::convert::TryInto;
use std::{borrow::Cow, time::SystemTime};
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use url::Url;
use websocket::{
    header::Headers,
};

/// Type of a `Callback` function. (Normal closures can be passed in here).
type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I) + 'static + Sync + Send>>>>;

/// An object that implements Transport
type DynamicTransport = dyn Transport + Sync + Send;

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone)]
pub struct EngineIoSocket<T: ?Sized + Transport = DynamicTransport> {
    transport: Arc<T>,
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

pub struct EngineIoSocketBuilder {
    url: Url,
    tls_config: Option<TlsConnector>,
    headers: Option<HeaderMap>
}

impl EngineIoSocketBuilder {
    pub fn new(url: Url) -> Self {
        EngineIoSocketBuilder {
            url,
            headers: None,
            tls_config: None
        }
    }

    pub fn set_tls_config(mut self, tls_config: TlsConnector) -> Self {
        self.tls_config=Some(tls_config);
        self
    }

    pub fn set_headers(mut self, headers: HeaderMap) -> Self {
        self.headers=Some(headers);
        self
    }

    pub fn build(&self) -> Result<EngineIoSocket> {

        let mut url = self.url.clone();
        let mut url = url
            .query_pairs_mut()
            .append_pair("EIO", "4")
            .finish();

        // No path add engine.io
        if url.path() == "/" {
            url.set_path("/engine.io/");
        }

        // Start with polling transport
        let transport = PollingTransport::new(
            url.clone(),
            self.tls_config.clone(),
            self.headers.clone(),
        );

        let handshake: HandshakePacket = Packet::try_from(transport.poll()?)?.try_into()?;

        // update the base_url with the new sid
        let url = url
            .query_pairs_mut()
            .append_pair("sid", &handshake.sid[..])
            .finish();

        let url = websocket::client::Url::parse(&url.to_string())?;

        // check if we could upgrade to websockets
        let websocket_upgrade = handshake
            .upgrades
            .iter()
            .any(|upgrade| upgrade.to_lowercase() == *"websocket");

        if websocket_upgrade {
            match url.scheme() {
                "https" => {
                    let transport = WebsocketSecureTransport::new(
                        url,
                        self.tls_config.clone(),
                        self.get_ws_headers()?,
                    );
                    match transport.upgrade() {
                        Ok(_) => return Ok(EngineIoSocket::new(transport, handshake)),
                        // TODO: Notify user that upgrade failed.
                        Err(_) => ()
                    }
                }
                "http" => {
                    let transport = WebsocketTransport::new(url, self.get_ws_headers()?);
                    match transport.upgrade() {
                        Ok(_) => return Ok(EngineIoSocket::new(transport, handshake)),
                        // TODO: Notify user that upgrade failed.
                        Err(_) => ()
                    }
                }
                _ => return Err(Error::InvalidUrlScheme(url.scheme().to_string())),
            }
        }
        // If we can't upgrade or upgrade fails use polling
        return Ok(EngineIoSocket::new(transport, handshake));
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

impl EngineIoSocket {
    pub fn new<T: Transport + Sync + Send + 'static>(transport: T, handshake: HandshakePacket) -> Self {
        EngineIoSocket {
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
    /// Registers an `on_open` callback.
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

    /// Registers an `on_error` callback.
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

    /// Registers an `on_packet` callback.
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

    /// Registers an `on_data` callback.
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

    /// Registers an `on_close` callback.
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

    /// Opens the connection to a specified server. Includes an opening `GET`
    /// request to the server, the server passes back the handshake data in the
    /// response. If the handshake data mentions a websocket upgrade possibility,
    /// we try to upgrade the connection. Afterwards a first Pong packet is sent
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

        if let Err(error) = self.transport.emit(data, is_binary_att) {
            self.call_error_callback(error.to_string())?;
            return Err(error);
        }

        Ok(())
    }

    /// Polls for next payload
    pub(crate) fn poll(&self) -> Result<Option<Payload>> {
        if self.connected.load(Ordering::Acquire) {
            let data = self.transport.poll()?;

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
            self.connection_data.ping_timeout + self.connection_data.ping_interval
        );

        while self.connected.load(Ordering::Acquire) {
            let packets = self.poll()?;

            // Double check that we have not disconnected since last loop.
            if !self.connected.load(Ordering::Acquire) {
                break;
            }

            if packets.is_none() {
                break;
            }

            for packet in packets.unwrap() {
                // check for the appropriate action or callback
                if let Some(on_packet) = self.on_packet.read()?.as_ref() {
                    spawn_scoped!(on_packet(packet.clone()));
                }
                match packet.packet_id {
                    PacketId::MessageBase64 => {
                        let on_data = self.on_data.read()?;
                        if let Some(function) = on_data.as_ref() {
                            spawn_scoped!(function(packet.data));
                        }
                        drop(on_data);
                    }
                    PacketId::Message => {
                        if let Some(on_data) = self.on_data.read()?.as_ref() {
                            spawn_scoped!(on_data(Bytes::from(packet)));
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

            if server_timeout < last_ping.elapsed() {
                // the server is unreachable
                // set current state to not connected and stop polling
                self.connected.store(false, Ordering::Release);
            }
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

    /// Binds the socket to a certain `address`. Attention! This doesn't allow
    /// to configure callbacks afterwards.
    pub fn bind(mut self) -> Result<()> {
        if self.connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionAfterOpen());
        }
        self.connect()?;

        thread::spawn(move || {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            loop {
                match self.poll_cycle() {
                    Ok(_) => break,
                    e @ Err(Error::IncompleteHttp(_)) | e @ Err(Error::IncompleteResponseFromReqwest(_)) => {
                        panic!("{}", e.unwrap_err())
                    }
                    _ => (),
                }
            }
        });
        Ok(())
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }

    /// Disconnects this client from the server by sending a `engine.io` closing
    /// packet.
    pub fn disconnect(&mut self) -> Result<()> {
        let packet = Packet::new(PacketId::Close, Bytes::from_static(&[]));
        self.connected.store(false, Ordering::Release);
        self.emit(packet, false)
    }
}

impl Debug for EngineIoSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocket(transport: ?, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_ping: {:?}, last_pong: {:?}, connection_data: {:?})",
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

    use std::{thread::sleep, time::Duration};

    use crate::engineio::packet::PacketId;

    use super::*;

    #[test]
    fn test_basic_connection() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let mut socket = EngineIoSocketBuilder::new(Url::parse(&url.to_string()).unwrap()).build()?;

        assert!(socket
            .on_open(|_| {
                println!("Open event!");
            })
            .is_ok());

        assert!(socket
            .on_packet(|packet| {
                println!("Received packet: {:?}", packet);
            })
            .is_ok());

        assert!(socket
            .on_data(|data| {
                println!("Received packet: {:?}", std::str::from_utf8(&data));
            })
            .is_ok());

        socket.connect()?;

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"Hello World"),),
                false
            )
            .is_ok());

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"Hello World2"),),
                false
            )
            .is_ok());

        assert!(socket
            .emit(Packet::new(PacketId::Pong, Bytes::new()), false)
            .is_ok());

        sleep(Duration::from_secs(26));

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"Hello World3"),),
                false
            )
            .is_ok());

        assert!(socket.bind().is_ok());
        Ok(())
    }

    #[test]
    fn test_illegal_actions() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let mut sut = EngineIoSocketBuilder::new(Url::parse(&url.to_string()).unwrap()).build()?;

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

        assert!(sut.bind().is_ok());

        let sut = EngineIoSocketBuilder::new(url).build()?;
        assert!(sut.poll_cycle().is_err());

        Ok(())
    }
    use reqwest::header::HOST;

    use crate::engineio::packet::Packet;

    use super::*;
    #[test]
    fn test_connection_polling() -> Result<()> {
        let mut socket = EngineIoSocket::new(true, None, None);

        let url = crate::engineio::test::engine_io_server()?;

        socket.open(url).unwrap();

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
                false,
            )
            .is_ok());

        socket.on_data = Arc::new(RwLock::new(Some(Box::new(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        }))));

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"Hi")),
                true
            )
            .is_ok());

        let mut sut = socket.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            sut.close();
        });

        assert!(socket.poll_cycle().is_ok());

        Ok(())
    }

    #[test]
    fn test_connection_secure_ws_http() -> Result<()> {
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
            let url = crate::engineio::test::engine_io_server_secure()?;

        let mut headers = HeaderMap::new();
        headers.insert(HOST, host.parse().unwrap());
        let mut builder = EngineIoSocketBuilder::new(Url::parse(&url.to_string()).unwrap());
        builder = builder.set_tls_config(TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap());
        builder = builder.set_headers(headers);
        let mut socket = builder.build()?;

        let url = crate::engineio::test::engine_io_server_secure()?;

        socket.connect().unwrap();

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
                false,
            )
            .is_ok());

        socket.on_data = Arc::new(RwLock::new(Some(Box::new(|data| {
            println!(
                "Received: {:?}",
                std::str::from_utf8(&data).expect("Error while decoding utf-8")
            );
        }))));

        // closes the connection
        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"CLOSE")),
                false,
            )
            .is_ok());

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"Hi")),
                true
            )
            .is_ok());

        let mut sut = socket.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            sut.close();
        });

        assert!(socket.poll_cycle().is_ok());
        Ok(())
    }

    #[test]
    fn test_open_invariants() -> Result<()> {
        let url = crate::engineio::test::engine_io_server()?;
        let illegal_url = "this is illegal";

        assert!(Url::parse(&illegal_url).is_err());

        let invalid_protocol = "file:///tmp/foo";
        assert!(EngineIoSocketBuilder::new(Url::parse(&invalid_protocol).unwrap()).build().is_err());

        let sut = EngineIoSocketBuilder::new(url).build()?;
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .expect_err("error");
        assert!(matches!(Error::IllegalActionBeforeOpen(), _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::new();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host.parse().unwrap());

        let _ = EngineIoSocketBuilder::new(url).set_tls_config(TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .unwrap()).build()?;
        let _ = EngineIoSocketBuilder::new(url).set_headers(headers).build()?;
        Ok(())
    }
}
