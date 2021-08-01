#![allow(unused)]
use std::{ops::Deref, thread};

use crate::engineio::packet::{decode_payload, encode_payload, Packet, PacketId};
use crate::error::{Error, Result};
use adler32::adler32;
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use reqwest::{
    blocking::{Client, ClientBuilder},
    header::HeaderMap,
    Url,
};
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, time::SystemTime};
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
    time::{Duration, Instant},
};
use websocket::{
    client::sync::Client as WsClient,
    dataframe::Opcode,
    header::Headers,
    receiver::Reader,
    sync::stream::{TcpStream, TlsStream},
    sync::Writer,
    ws::dataframe::DataFrame,
    ClientBuilder as WsClientBuilder, Message,
};

/// Type of a `Callback` function. (Normal closures can be passed in here).
type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I) + 'static + Sync + Send>>>>;

/// The different types of transport used for transmitting a payload.
#[derive(Clone)]
enum TransportType {
    Polling(Arc<Mutex<Client>>),
    Websocket(Arc<Mutex<Reader<TcpStream>>>, Arc<Mutex<Writer<TcpStream>>>),
    SecureWebsocket(Arc<Mutex<WsClient<TlsStream<TcpStream>>>>),
}

impl Debug for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "TransportType({})",
            match self {
                Self::Polling(_) => "Polling",
                Self::Websocket(_, _) | Self::SecureWebsocket(_) => "Websocket",
            }
        ))
    }
}

//TODO: Move this to packet
/// Data which gets exchanged in a handshake as defined by the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct HandshakeData {
    sid: String,
    upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    ping_interval: u64,
    #[serde(rename = "pingTimeout")]
    ping_timeout: u64,
}

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone)]
pub struct EngineSocket {
    transport: Arc<TransportType>,
    pub on_error: Callback<String>,
    pub on_open: Callback<()>,
    pub on_close: Callback<()>,
    pub on_data: Callback<Bytes>,
    pub on_packet: Callback<Packet>,
    pub connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    host_address: Arc<Mutex<Option<String>>>,
    tls_config: Arc<Option<TlsConnector>>,
    custom_headers: Arc<Option<HeaderMap>>,
    connection_data: Arc<RwLock<Option<HandshakeData>>>,
    engine_io_mode: Arc<AtomicBool>,
}

impl EngineSocket {
    /// Creates an instance of `EngineSocket`.
    pub fn new(
        engine_io_mode: bool,
        tls_config: Option<TlsConnector>,
        opening_headers: Option<HeaderMap>,
    ) -> Self {
        let client = match (tls_config.clone(), opening_headers.clone()) {
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

        EngineSocket {
            transport: Arc::new(TransportType::Polling(Arc::new(Mutex::new(client)))),
            on_error: Arc::new(RwLock::new(None)),
            on_open: Arc::new(RwLock::new(None)),
            on_close: Arc::new(RwLock::new(None)),
            on_data: Arc::new(RwLock::new(None)),
            on_packet: Arc::new(RwLock::new(None)),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            host_address: Arc::new(Mutex::new(None)),
            tls_config: Arc::new(tls_config),
            custom_headers: Arc::new(opening_headers),
            connection_data: Arc::new(RwLock::new(None)),
            engine_io_mode: Arc::new(AtomicBool::from(engine_io_mode)),
        }
    }

    /// Binds the socket to a certain `address`. Attention! This doesn't allow
    /// to configure callbacks afterwards.
    pub fn bind<T: Into<String>>(&mut self, address: T) -> Result<()> {
        if self.connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionAfterOpen());
        }
        self.open(address.into())?;

        // TODO: Refactor this
        let mut s = self.clone();
        thread::spawn(move || {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            loop {
                match s.poll_cycle() {
                    Ok(_) => break,
                    e @ Err(Error::IncompleteHttp(_))
                    | e @ Err(Error::IncompleteResponseFromReqwest(_)) => {
                        panic!("{}", e.unwrap_err())
                    }
                    _ => (),
                }
            }
        });

        Ok(())
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

    /// Opens the connection to a specified server. Includes an opening `GET`
    /// request to the server, the server passes back the handshake data in the
    /// response. If the handshake data mentions a websocket upgrade possibility,
    /// we try to upgrade the connection. Afterwards a first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub fn open<T: Into<String> + Clone>(&mut self, address: T) -> Result<()> {
        // build the query path, random_t is used to prevent browser caching
        let query_path = self.get_query_path()?;

        match &mut self.transport.as_ref() {
            TransportType::Polling(client) => {
                if let Ok(mut full_address) = Url::parse(&(address.to_owned().into() + &query_path))
                {
                    self.host_address = Arc::new(Mutex::new(Some(address.into())));

                    // change the url scheme here if necessary, unwrapping here is
                    // safe as both http and https are valid schemes for sure
                    match full_address.scheme() {
                        "ws" => full_address.set_scheme("http").unwrap(),
                        "wss" => full_address.set_scheme("https").unwrap(),
                        "http" | "https" => (),
                        _ => return Err(Error::InvalidUrl(full_address.to_string())),
                    }

                    let response = client.lock()?.get(full_address).send()?.text()?;

                    // the response contains the handshake data
                    if let Ok(conn_data) = serde_json::from_str::<HandshakeData>(&response[1..]) {
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
                        let function = self.on_open.read()?;
                        if let Some(function) = function.as_ref() {
                            spawn_scoped!(function(()));
                        }
                        drop(function);

                        // set the last ping to now and set the connected state
                        *self.last_ping.lock()? = Instant::now();

                        // emit a pong packet to keep trigger the ping cycle on the server
                        self.emit(Packet::new(PacketId::Pong, Bytes::new()), false)?;

                        return Ok(());
                    }

                    let error = Error::InvalidHandshake(response);
                    self.call_error_callback(format!("{}", error))?;
                    return Err(error);
                }

                let error = Error::InvalidUrl(address.into());
                self.call_error_callback(format!("{}", error))?;
                Err(error)
            }
            _ => unreachable!("Can't happen as we initialize with polling and upgrade later"),
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

            if self.engine_io_mode.load(Ordering::Relaxed) {
                address.set_path(&(address.path().to_owned() + "engine.io/"));
            } else {
                address.set_path(&(address.path().to_owned() + "socket.io/"));
            }

            let full_address = address
                .query_pairs_mut()
                .clear()
                .append_pair("EIO", "4")
                .append_pair("transport", "websocket")
                .append_pair("sid", new_sid)
                .finish();

            let custom_headers = self.get_headers_from_header_map();
            match full_address.scheme() {
                "https" => {
                    self.perform_upgrade_secure(&full_address, &custom_headers)?;
                }
                "http" => {
                    self.perform_upgrade_insecure(&full_address, &custom_headers)?;
                }
                _ => return Err(Error::InvalidUrl(full_address.to_string())),
            }

            return Ok(());
        }
        Err(Error::InvalidHandshake("Error - invalid url".to_owned()))
    }

    /// Converts between `reqwest::HeaderMap` and `websocket::Headers`, if they're currently set, if not
    /// An empty (allocation free) header map is returned.
    fn get_headers_from_header_map(&mut self) -> Headers {
        let mut headers = Headers::new();
        // SAFETY: unwrapping is safe as we only hand out `Weak` copies after the connection procedure
        if let Some(map) = Arc::get_mut(&mut self.custom_headers).unwrap().take() {
            for (key, val) in map {
                if let Some(h_key) = key {
                    headers.append_raw(h_key.to_string(), val.as_bytes().to_owned());
                }
            }
        }
        headers
    }

    /// Performs the socketio upgrade handshake via `wss://`. A Description of the
    /// upgrade request can be found above.
    fn perform_upgrade_secure(&mut self, address: &Url, headers: &Headers) -> Result<()> {
        // connect to the server via websockets
        // SAFETY: unwrapping is safe as we only hand out `Weak` copies after the connection procedure
        let tls_config = Arc::get_mut(&mut self.tls_config).unwrap().take();
        let mut client = WsClientBuilder::new(address.as_ref())?
            .custom_headers(headers)
            .connect_secure(tls_config)?;

        client.set_nonblocking(false)?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        client.send_message(&Message::text(Cow::Borrowed("2probe")))?;

        // expect to receive a probe packet
        let message = client.recv_message()?;
        if message.take_payload() != b"3probe" {
            return Err(Error::InvalidHandshake("Error".to_owned()));
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        client.send_message(&Message::text(Cow::Borrowed("5")))?;

        // upgrade the transport layer
        // SAFETY: unwrapping is safe as we only hand out `Weak` copies after the connection
        // procedure
        *Arc::get_mut(&mut self.transport).unwrap() =
            TransportType::SecureWebsocket(Arc::new(Mutex::new(client)));

        Ok(())
    }

    /// Performs the socketio upgrade handshake in an via `ws://`. A Description of the
    /// upgrade request can be found above.
    fn perform_upgrade_insecure(&mut self, addres: &Url, headers: &Headers) -> Result<()> {
        // connect to the server via websockets
        let client = WsClientBuilder::new(addres.as_ref())?
            .custom_headers(headers)
            .connect_insecure()?;
        client.set_nonblocking(false)?;

        let (mut receiver, mut sender) = client.split()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        sender.send_message(&Message::text(Cow::Borrowed("2probe")))?;

        // expect to receive a probe packet
        let message = receiver.recv_message()?;
        if message.take_payload() != b"3probe" {
            return Err(Error::InvalidHandshake("Error".to_owned()));
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        sender.send_message(&Message::text(Cow::Borrowed("5")))?;

        // upgrade the transport layer
        // SAFETY: unwrapping is safe as we only hand out `Weak` copies after the connection
        // procedure
        *Arc::get_mut(&mut self.transport).unwrap() =
            TransportType::Websocket(Arc::new(Mutex::new(receiver)), Arc::new(Mutex::new(sender)));

        Ok(())
    }

    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    pub fn emit(&self, packet: Packet, is_binary_att: bool) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
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

        match &self.transport.as_ref() {
            TransportType::Polling(client) => {
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

                let client = client.lock()?;
                let status = client
                    .post(address)
                    .body(data_to_send)
                    .send()?
                    .status()
                    .as_u16();

                drop(client);

                if status != 200 {
                    let error = Error::IncompleteHttp(status);
                    self.call_error_callback(format!("{}", error))?;
                    return Err(error);
                }

                Ok(())
            }
            TransportType::Websocket(_, writer) => {
                let mut writer = writer.lock()?;

                let message = if is_binary_att {
                    Message::binary(Cow::Borrowed(data.as_ref()))
                } else {
                    Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
                };
                writer.send_message(&message)?;

                Ok(())
            }
            TransportType::SecureWebsocket(client) => {
                let mut client = client.lock()?;

                let message = if is_binary_att {
                    Message::binary(Cow::Borrowed(data.as_ref()))
                } else {
                    Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
                };

                client.send_message(&message)?;

                Ok(())
            }
        }
    }

    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    pub fn poll_cycle(&mut self) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }
        let client = Client::new();

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
            let data = match &self.transport.as_ref() {
                // we won't use the shared client as this blocks the resource
                // in the long polling requests
                TransportType::Polling(_) => {
                    let query_path = self.get_query_path()?;

                    let host = self.host_address.lock()?;
                    let address =
                        Url::parse(&(host.as_ref().unwrap().to_owned() + &query_path)[..]).unwrap();
                    drop(host);

                    client.get(address).send()?.bytes()?
                }
                TransportType::Websocket(receiver, _) => {
                    let mut receiver = receiver.lock()?;

                    // if this is a binary payload, we mark it as a message
                    let received_df = receiver.recv_dataframe()?;
                    match received_df.opcode {
                        Opcode::Binary => {
                            let mut message = BytesMut::with_capacity(received_df.data.len() + 1);
                            message.put_u8(b'4');
                            message.put(received_df.take_payload().as_ref());

                            message.freeze()
                        }
                        _ => Bytes::from(received_df.take_payload()),
                    }
                }
                TransportType::SecureWebsocket(client) => {
                    let mut client = client.lock()?;

                    // if this is a binary payload, we mark it as a message
                    let received_df = client.recv_dataframe()?;
                    match received_df.opcode {
                        Opcode::Binary => {
                            let mut message = BytesMut::with_capacity(received_df.data.len() + 1);
                            message.put_u8(b'4');
                            message.put(received_df.take_payload().as_ref());

                            message.freeze()
                        }
                        _ => Bytes::from(received_df.take_payload()),
                    }
                }
            };

            if data.is_empty() {
                return Ok(());
            }

            let packets = decode_payload(data)?;

            for packet in packets {
                {
                    let on_packet = self.on_packet.read()?;
                    // call the packet callback
                    if let Some(function) = on_packet.as_ref() {
                        spawn_scoped!(function(packet.clone()));
                    }
                }

                // check for the appropriate action or callback
                match packet.packet_id {
                    PacketId::Message => {
                        let on_data = self.on_data.read()?;
                        if let Some(function) = on_data.as_ref() {
                            spawn_scoped!(function(packet.data));
                        }
                        drop(on_data);
                    }

                    PacketId::Close => {
                        let on_close = self.on_close.read()?;
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

    /// Produces a random String that is used to prevent browser caching.
    #[inline]
    fn get_random_t() -> String {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        hash.to_string()
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

    // Constructs the path for a request, depending on the different situations.
    #[inline]
    fn get_query_path(&self) -> Result<String> {
        // build the base path
        let mut path = format!(
            "/{}/?EIO=4&transport={}&t={}",
            if self.engine_io_mode.load(Ordering::Relaxed) {
                "engine.io"
            } else {
                "socket.io"
            },
            match self.transport.as_ref() {
                TransportType::Polling(_) => "polling",
                TransportType::Websocket(_, _) | TransportType::SecureWebsocket(_) => "websocket",
            },
            EngineSocket::get_random_t(),
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

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }
}

impl Debug for EngineSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocket(transport: {:?}, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_ping: {:?}, last_pong: {:?}, host_address: {:?}, tls_config: {:?}, connection_data: {:?}, engine_io_mode: {:?})",
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
            self.host_address,
            self.tls_config,
            self.connection_data,
            self.engine_io_mode,
        ))
    }
}

#[cfg(test)]
mod test {

    use std::{thread::sleep, time::Duration};

    use crate::engineio::packet::PacketId;

    use super::*;

    const SERVER_URL: &str = "http://localhost:4201";

    #[test]
    fn test_basic_connection() {
        let mut socket = EngineSocket::new(true, None, None);

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

        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

        assert!(socket.bind(url).is_ok());

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
    }

    #[test]
    fn test_illegal_actions() {
        let mut sut = EngineSocket::new(true, None, None);

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .is_err());
        assert!(sut
            .emit(
                Packet::new(PacketId::Message, Bytes::from_static(b"")),
                true
            )
            .is_err());

        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

        assert!(sut.bind(url).is_ok());

        assert!(sut.on_open(|_| {}).is_err());
        assert!(sut.on_close(|_| {}).is_err());
        assert!(sut.on_packet(|_| {}).is_err());
        assert!(sut.on_data(|_| {}).is_err());
        assert!(sut.on_error(|_| {}).is_err());

        let mut sut = EngineSocket::new(true, None, None);
        assert!(sut.poll_cycle().is_err());
    }
    use reqwest::header::HOST;

    use crate::engineio::packet::Packet;

    use super::*;
    /// The `engine.io` server for testing runs on port 4201
    const SERVER_URL_SECURE: &str = "https://localhost:4202";

    #[test]
    fn test_connection_polling() {
        let mut socket = EngineSocket::new(true, None, None);

        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

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
    fn test_connection_secure_ws_http() {
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());

        let mut headers = HeaderMap::new();
        headers.insert(HOST, host.parse().unwrap());
        let mut socket = EngineSocket::new(
            true,
            Some(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap(),
            ),
            Some(headers),
        );

        let url = std::env::var("ENGINE_IO_SECURE_SERVER")
            .unwrap_or_else(|_| SERVER_URL_SECURE.to_owned());

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
        let mut sut = EngineSocket::new(false, None, None);
        let illegal_url = "this is illegal";

        let _error = sut.open(illegal_url).expect_err("Error");
        assert!(matches!(
            Error::InvalidUrl(String::from("this is illegal")),
            _error
        ));

        let mut sut = EngineSocket::new(false, None, None);
        let invalid_protocol = "file:///tmp/foo";

        let _error = sut.open(invalid_protocol).expect_err("Error");
        assert!(matches!(
            Error::InvalidUrl(String::from("file://localhost:4200")),
            _error
        ));

        let sut = EngineSocket::new(false, None, None);
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")), false)
            .expect_err("error");
        assert!(matches!(Error::IllegalActionBeforeOpen(), _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::new();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host.parse().unwrap());

        let _ = EngineSocket::new(
            true,
            Some(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap(),
            ),
            None,
        );

        let _ = EngineSocket::new(true, None, Some(headers));
    }
}
