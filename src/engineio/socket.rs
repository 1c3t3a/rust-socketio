use crate::client::Client;
use crate::engineio::packet::{decode_payload, encode_payload, HandshakePacket, Packet, PacketId};
use crate::engineio::transport_emitter::{EventEmitter, TransportEmitter};
use crate::engineio::transports::Transport;
use crate::error::{Error, Result};
use adler32::adler32;
use bytes::Bytes;
use native_tls::TlsConnector;
use reqwest::{header::HeaderMap, Url};
use std::convert::TryInto;
use std::time::SystemTime;
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
    time::{Duration, Instant},
};

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone, Debug)]
pub struct EngineIOSocket {
    pub(super) transport: Arc<RwLock<TransportEmitter>>,
    pub connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    host_address: Arc<Mutex<Option<String>>>,
    connection_data: Arc<RwLock<Option<HandshakePacket>>>,
    root_path: Arc<RwLock<String>>,
}

impl EngineIOSocket {
    /// Creates an instance of `EngineIOSocket`.
    pub fn new(
        root_path: Option<String>,
        tls_config: Option<TlsConnector>,
        opening_headers: Option<HeaderMap>,
    ) -> Self {
        EngineIOSocket {
            transport: Arc::new(RwLock::new(TransportEmitter::new(
                tls_config,
                opening_headers,
            ))),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            host_address: Arc::new(Mutex::new(None)),
            connection_data: Arc::new(RwLock::new(None)),
            root_path: Arc::new(RwLock::new(
                root_path.unwrap_or_else(|| "engine.io".to_owned()),
            )),
        }
    }

    /// This handles the upgrade from polling to websocket transport. Looking at the protocol
    /// specs, this needs the following preconditions:
    /// - the handshake must have already happened
    /// - the handshake `upgrade` field needs to contain the value `websocket`.
    /// If those preconditions are valid, it is save to call the method. The protocol then
    /// requires the following procedure:
    /// - the client starts to connect to the server via websocket, mentioning the related `sid` received
    ///   in the handshake.
    /// - to test out the connection, the client sends a `ping` packet with the payload `probe`.
    /// - if the server receives this correctly, it responses by sending a `pong` packet with the payload `probe`.
    /// - if the client receives that both sides can be sure that the upgrade is save and the client requests
    ///   the final upgrade by sending another `update` packet over `websocket`.
    /// If this procedure is alright, the new transport is established.
    fn upgrade_connection(&mut self, new_sid: &str) -> Result<()> {
        let host_address = self.host_address.lock()?;
        if let Some(address) = &*host_address {
            // unwrapping here is in fact save as we only set this url if it gets orderly parsed
            let mut address = Url::parse(&address).unwrap();
            drop(host_address);

            address
                .set_path(&(address.path().to_owned() + self.root_path.read()?[..].as_ref() + "/"));

            let full_address = address
                .query_pairs_mut()
                .clear()
                .append_pair("EIO", "4")
                .append_pair("transport", "websocket")
                .append_pair("sid", new_sid)
                .finish();

            match full_address.scheme() {
                "https" => {
                    self.transport
                        .write()?
                        .upgrade_websocket_secure(full_address.to_string())?;
                }
                "http" => {
                    self.transport
                        .write()?
                        .upgrade_websocket(full_address.to_string())?;
                }
                _ => return Err(Error::InvalidUrl(full_address.to_string())),
            }

            return Ok(());
        }
        Err(Error::HandshakeError("Error - invalid url".to_owned()))
    }

    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    pub fn emit(&self, packet: Packet, is_binary_att: bool) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::ActionBeforeOpen;
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }
        // build the target address
        let query_path = self.get_query_path()?;

        let host = self.host_address.lock()?;
        let address = Url::parse(&(host.as_ref().unwrap().to_owned() + &query_path)[..]).unwrap();
        drop(host);

        // send a post request with the encoded payload as body
        // if this is a binary attachment, then send the raw bytes
        let data = if is_binary_att {
            packet.data
        } else {
            encode_payload(vec![packet])
        };

        if let Err(error) = self
            .transport
            .read()?
            .emit(address.to_string(), data, is_binary_att)
        {
            self.call_error_callback(error.to_string())?;
            return Err(error);
        }

        Ok(())
    }

    /// Produces a random String that is used to prevent browser caching.
    #[inline]
    fn get_hashed_time(&self) -> String {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        hash.to_string()
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }

    // Constructs the path for a request, depending on the different situations.
    #[inline]
    fn get_query_path(&self) -> Result<String> {
        // build the base path
        let mut path = format!(
            "/{}/?EIO=4&transport={}&t={}",
            self.root_path.read()?,
            self.transport.read()?.get_transport_name()?,
            self.get_hashed_time(),
        );

        // append a session id if the socket is connected
        // unwrapping is safe as a connected socket always
        // holds its connection data
        if self.connected.load(Ordering::Acquire) {
            path.push_str(&format!(
                "&sid={}",
                (*self.connection_data).read()?.as_ref().unwrap().sid
            ));
        }

        Ok(path)
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) -> Result<()> {
        let transport = self.transport.read()?;
        let function = transport.on_error.read()?;
        if let Some(function) = function.as_ref() {
            spawn_scoped!(function(text));
        }
        drop(function);

        Ok(())
    }
}

impl EventEmitter for EngineIOSocket {
    fn set_on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        self.transport.write()?.set_on_open(function)
    }

    fn set_on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        self.transport.write()?.set_on_error(function)
    }

    fn set_on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        self.transport.write()?.set_on_packet(function)
    }

    fn set_on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send,
    {
        self.transport.write()?.set_on_data(function)
    }

    fn set_on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        self.transport.write()?.set_on_close(function)
    }
}

impl Client for EngineIOSocket {
    /// Opens the connection to a specified server. Includes an opening `GET`
    /// request to the server, the server passes back the handshake data in the
    /// response. If the handshake data mentions a websocket upgrade possibility,
    /// we try to upgrade the connection. Afterwards a first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    fn connect<T: Into<String> + Clone>(&mut self, address: T) -> Result<()> {
        // build the query path, random_t is used to prevent browser caching
        let query_path = self.get_query_path()?;

        if let Ok(mut full_address) = Url::parse(&(address.to_owned().into() + &query_path)) {
            self.host_address = Arc::new(Mutex::new(Some(address.into())));

            // change the url scheme here if necessary, unwrapping here is
            // safe as both http and https are valid schemes for sure
            match full_address.scheme() {
                "ws" => full_address.set_scheme("http").unwrap(),
                "wss" => full_address.set_scheme("https").unwrap(),
                "http" | "https" => (),
                _ => return Err(Error::InvalidUrl(full_address.to_string())),
            }

            let response = self.transport.read()?.poll(full_address.to_string())?;

            let handshake: Result<HandshakePacket> = Packet::decode(response.clone())?.try_into();

            // the response contains the handshake data
            if let Ok(conn_data) = handshake {
                self.connected.store(true, Ordering::Release);

                // check if we could upgrade to websockets
                let websocket_upgrade = conn_data
                    .upgrades
                    .iter()
                    .any(|upgrade| upgrade.to_lowercase() == *"websocket");

                // if we have an upgrade option, send the corresponding request, if this doesn't work
                // for some reason, proceed via polling
                if websocket_upgrade {
                    let _ = self.upgrade_connection(&conn_data.sid);
                }

                // set the connection data
                let mut connection_data = self.connection_data.write()?;
                *connection_data = Some(conn_data);
                drop(connection_data);

                // call the on_open callback
                let transport = self.transport.read()?;
                let function = transport.on_open.read()?;
                if let Some(function) = function.as_ref() {
                    spawn_scoped!(function(()));
                }
                drop(function);
                drop(transport);

                // set the last ping to now and set the connected state
                *self.last_ping.lock()? = Instant::now();

                let _test = self.transport.is_poisoned();

                // emit a pong packet to keep trigger the ping cycle on the server
                self.emit(Packet::new(PacketId::Pong, Bytes::new()), false)?;

                Ok(())
            } else {
                let error =
                    Error::HandshakeError(std::str::from_utf8(response[..].as_ref())?.to_owned());
                self.call_error_callback(format!("{}", error))?;
                Err(error)
            }
        } else {
            let error = Error::InvalidUrl(address.into());
            self.call_error_callback(format!("{}", error))?;
            Err(error)
        }
    }

    /// Disconnects this client from the server by sending a `engine.io` closing
    /// packet.
    fn disconnect(&mut self) -> Result<()> {
        let packet = Packet::new(PacketId::Close, Bytes::from_static(&[]));
        self.connected.store(false, Ordering::Release);
        self.emit(packet, false)
    }
}

/// EngineSocket related functions that use client side logic
pub trait EngineClient {
    fn poll_cycle(&self) -> Result<()>;
}

impl EngineClient for EngineIOSocket {
    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    fn poll_cycle(&self) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::ActionBeforeOpen;
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }

        // as we don't have a mut self, the last_ping needs to be safed for later
        let mut last_ping = *self.last_ping.lock()?;
        // the time after we assume the server to be timed out
        let server_timeout = Duration::from_millis(
            (*self.connection_data)
                .read()?
                .as_ref()
                .map_or_else(|| 0, |data| data.ping_timeout)
                + (*self.connection_data)
                    .read()?
                    .as_ref()
                    .map_or_else(|| 0, |data| data.ping_interval),
        );

        while self.connected.load(Ordering::Acquire) {
            let query_path = self.get_query_path()?.to_owned();

            let host = self.host_address.lock()?;
            let address =
                Url::parse(&(host.as_ref().unwrap().to_owned() + &query_path)[..]).unwrap();
            drop(host);
            let transport = self.transport.read()?;
            let data = transport.poll(address.to_string())?;
            drop(transport);

            if data.is_empty() {
                return Ok(());
            }

            let packets = decode_payload(data)?;

            for packet in packets {
                {
                    let transport = self.transport.read()?;
                    let on_packet = transport.on_packet.read()?;
                    // call the packet callback
                    if let Some(function) = on_packet.as_ref() {
                        spawn_scoped!(function(packet.clone()));
                    }
                }

                // check for the appropriate action or callback
                match packet.packet_id {
                    PacketId::Message => {
                        let transport = self.transport.read()?;
                        let on_data = transport.on_data.read()?;
                        if let Some(function) = on_data.as_ref() {
                            spawn_scoped!(function(packet.data));
                        }
                        drop(on_data);
                    }

                    PacketId::Close => {
                        let transport = self.transport.read()?;
                        let on_close = transport.on_close.read()?;
                        if let Some(function) = on_close.as_ref() {
                            spawn_scoped!(function(()));
                        }
                        drop(on_close);
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
}

#[cfg(test)]
mod test {
    use reqwest::header::HOST;

    use crate::engineio::packet::{Packet, PacketId};
    use native_tls::Certificate;
    use std::fs::File;
    use std::io::Read;

    use super::*;
    /// The `engine.io` server for testing runs on port 4201
    const SERVER_URL: &str = "http://localhost:4201";
    const SERVER_URL_SECURE: &str = "https://localhost:4202";
    const CERT_PATH: &str = "ci/cert/ca.crt";

    #[test]
    fn test_connection_polling() {
        let mut socket = EngineIOSocket::new(None, None, None);

        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

        socket.connect(url).unwrap();

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
                false,
            )
            .is_ok());

        socket
            .set_on_data(|data| {
                println!(
                    "Received: {:?}",
                    std::str::from_utf8(&data).expect("Error while decoding utf-8")
                );
            })
            .unwrap();

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

        assert!(socket.poll_cycle().is_ok());
    }

    fn get_tls_connector() -> Result<TlsConnector> {
        let cert_path = std::env::var("CA_CERT_PATH").unwrap_or_else(|_| CERT_PATH.to_owned());
        let mut cert_file = File::open(cert_path)?;
        let mut buf = vec![];
        cert_file.read_to_end(&mut buf)?;
        let cert: Certificate = Certificate::from_pem(&buf[..]).unwrap();
        Ok(TlsConnector::builder()
            .add_root_certificate(cert)
            .build()
            .unwrap())
    }

    #[test]
    fn test_connection_secure_ws_http() {
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());

        let mut headers = HeaderMap::new();
        headers.insert(HOST, host.parse().unwrap());
        let mut socket =
            EngineIOSocket::new(None, Some(get_tls_connector().unwrap()), Some(headers));

        let url = std::env::var("ENGINE_IO_SECURE_SERVER")
            .unwrap_or_else(|_| SERVER_URL_SECURE.to_owned());

        socket.connect(url).unwrap();

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"HelloWorld")),
                false,
            )
            .is_ok());

        socket
            .set_on_data(|data| {
                println!(
                    "Received: {:?}",
                    std::str::from_utf8(&data).expect("Error while decoding utf-8")
                );
            })
            .unwrap();

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

        assert!(socket.poll_cycle().is_ok());
    }

    #[test]
    fn test_open_invariants() {
        let mut sut = EngineIOSocket::new(None, None, None);
        let illegal_url = "this is illegal";

        let _error = sut.connect(illegal_url).expect_err("Error");
        assert!(matches!(
            Error::InvalidUrl(String::from("this is illegal")),
            _error
        ));

        let mut sut = EngineIOSocket::new(None, None, None);
        let invalid_protocol = "file:///tmp/foo";

        let _error = sut.connect(invalid_protocol).expect_err("Error");
        assert!(matches!(
            Error::InvalidUrl(String::from("file://localhost:4200")),
            _error
        ));

        let sut = EngineIOSocket::new(None, None, None);
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .expect_err("error");
        assert!(matches!(Error::ActionBeforeOpen, _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::new();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host.parse().unwrap());

        let _ = EngineIOSocket::new(
            None,
            Some(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap(),
            ),
            None,
        );

        let _ = EngineIOSocket::new(None, None, Some(headers));
    }

    #[test]
    fn test_illegal_actions() {
        let sut = EngineIOSocket::new(None, None, None);
        assert!(sut.poll_cycle().is_err());

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .is_err());
        assert!(sut
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"")),
                true
            )
            .is_err());
    }

    use std::{thread::sleep, time::Duration};

    #[test]
    fn test_basic_connection() {
        let mut socket = EngineIOSocket::new(None, None, None);

        assert!(socket
            .set_on_open(|_| {
                println!("Open event!");
            })
            .is_ok());

        assert!(socket
            .set_on_packet(|packet| {
                println!("Received packet: {:?}", packet);
            })
            .is_ok());

        assert!(socket
            .set_on_data(|data| {
                println!("Received packet: {:?}", std::str::from_utf8(&data));
            })
            .is_ok());

        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

        assert!(socket.connect(url).is_ok());

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

        sleep(Duration::from_secs(2));

        assert!(socket
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"Hello World3"),),
                false
            )
            .is_ok());
    }
}
