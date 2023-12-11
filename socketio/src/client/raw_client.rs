use super::callback::Callback;
use crate::packet::{Packet, PacketId};
use crate::Error;
pub(crate) use crate::{event::Event, payload::Payload};
use rand::{thread_rng, Rng};
use serde_json::Value;

use crate::client::callback::{SocketAnyCallback, SocketCallback};
use crate::error::Result;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::Instant;

use crate::socket::Socket as InnerSocket;

/// Represents an `Ack` as given back to the caller. Holds the internal `id` as
/// well as the current ack'ed state. Holds data which will be accessible as
/// soon as the ack'ed state is set to true. An `Ack` that didn't get ack'ed
/// won't contain data.
#[derive(Debug)]
pub struct Ack {
    pub id: i32,
    timeout: Duration,
    time_started: Instant,
    callback: Callback<SocketCallback>,
}

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the server will connect to the default namespace `"/"`.
#[derive(Clone)]
pub struct RawClient {
    /// The inner socket client to delegate the methods to.
    socket: InnerSocket,
    on: Arc<Mutex<HashMap<Event, Callback<SocketCallback>>>>,
    on_any: Arc<Mutex<Option<Callback<SocketAnyCallback>>>>,
    outstanding_acks: Arc<Mutex<Vec<Ack>>>,
    // namespace, for multiplexing messages
    nsp: String,
    // Data send in the opening packet (commonly used as for auth)
    auth: Option<Value>,
}

impl RawClient {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new<T: Into<String>>(
        socket: InnerSocket,
        namespace: T,
        on: Arc<Mutex<HashMap<Event, Callback<SocketCallback>>>>,
        on_any: Arc<Mutex<Option<Callback<SocketAnyCallback>>>>,
        auth: Option<Value>,
    ) -> Result<Self> {
        Ok(RawClient {
            socket,
            nsp: namespace.into(),
            on,
            on_any,
            outstanding_acks: Arc::new(Mutex::new(Vec::new())),
            auth,
        })
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server. Attention: it's not allowed to add a
    /// callback after a call to this method.
    pub(crate) fn connect(&self) -> Result<()> {
        // Connect the underlying socket
        self.socket.connect()?;

        let auth = self.auth.as_ref().map(|data| data.to_string());

        // construct the opening packet
        let open_packet = Packet::new(PacketId::Connect, self.nsp.clone(), auth, None, 0, None);

        self.socket.send(open_packet)?;

        Ok(())
    }

    /// Sends a message to the server using the underlying `engine.io` protocol.
    /// This message takes an event, which could either be one of the common
    /// events like "message" or "error" or a custom event like "foo". But be
    /// careful, the data string needs to be valid JSON. It's recommended to use
    /// a library like `serde_json` to serialize the data properly.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::{ClientBuilder, RawClient, Payload};
    /// use serde_json::json;
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("test", |payload: Payload, socket: RawClient, _id: Option<i32>| {
    ///         println!("Received: {:#?}", payload);
    ///         socket.emit("test", json!({"hello": true})).expect("Server unreachable");
    ///      })
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// let json_payload = json!({"token": 123});
    ///
    /// let result = socket.emit("foo", json_payload);
    ///
    /// assert!(result.is_ok());
    /// ```
    #[inline]
    pub fn emit<E, D>(&self, event: E, data: D) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        self.socket.emit(&self.nsp, event.into(), data.into())
    }

    /// Disconnects this client from the server by sending a `socket.io` closing
    /// packet.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload, RawClient};
    /// use serde_json::json;
    ///
    /// fn handle_test(payload: Payload, socket: RawClient, _id: Option<i32>) {
    ///     println!("Received: {:#?}", payload);
    ///     socket.emit("test", json!({"hello": true})).expect("Server unreachable");
    /// }
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("test", handle_test)
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// let json_payload = json!({"token": 123});
    ///
    /// socket.emit("foo", json_payload);
    ///
    /// // disconnect from the server
    /// socket.disconnect();
    ///
    /// ```
    pub fn disconnect(&self) -> Result<()> {
        let disconnect_packet =
            Packet::new(PacketId::Disconnect, self.nsp.clone(), None, None, 0, None);

        // TODO: logging
        let _ = self.socket.send(disconnect_packet);
        self.socket.disconnect()?;

        let _ = self.callback(&Event::Close, "", None); // trigger on_close
        Ok(())
    }

    /// Sends a message to the server but `alloc`s an `ack` to check whether the
    /// server responded in a given time span. This message takes an event, which
    /// could either be one of the common events like "message" or "error" or a
    /// custom event like "foo", as well as a data parameter. But be careful,
    /// in case you send a [`Payload::String`], the string needs to be valid JSON.
    /// It's even recommended to use a library like serde_json to serialize the data properly.
    /// It also requires a timeout `Duration` in which the client needs to answer.
    /// If the ack is acked in the correct time span, the specified callback is
    /// called. The callback consumes a [`Payload`] which represents the data send
    /// by the server.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::{ClientBuilder, Payload, RawClient};
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use std::thread::sleep;
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("foo", |payload: Payload, _, _| println!("Received: {:#?}", payload))
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// let ack_callback = |message: Payload, socket: RawClient| {
    ///     match message {
    ///         Payload::Text(values) => println!("{:#?}", values),
    ///         Payload::Binary(bytes) => println!("Received bytes: {:#?}", bytes),
    ///         // This is deprecated, use Payload::Text instead
    ///         Payload::String(str) => println!("{}", str),
    ///    }
    /// };
    ///
    /// let payload = json!({"token": 123});
    /// socket.emit_with_ack("foo", payload, Duration::from_secs(2), ack_callback).unwrap();
    ///
    /// sleep(Duration::from_secs(2));
    /// ```
    #[inline]
    pub fn emit_with_ack<F, E, D>(
        &self,
        event: E,
        data: D,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: FnMut(Payload, RawClient, Option<i32>) + 'static + Send,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let id = thread_rng().gen_range(0..999);
        let socket_packet =
            self.socket
                .build_packet_for_payload(data.into(), event.into(), &self.nsp, Some(id), false)?;

        let ack = Ack {
            id,
            time_started: Instant::now(),
            timeout,
            callback: Callback::<SocketCallback>::new(callback),
        };

        // add the ack to the tuple of outstanding acks
        self.outstanding_acks.lock()?.push(ack);

        self.socket.send(socket_packet)?;
        Ok(())
    }

    pub fn emit_answer<D>(
        &self,
        id: Option<i32>,
        data: D,
    ) -> Result<()>
    where
        D: Into<Payload>,
    {
        let id = match id {
            None => {
                return Err(Error::MissedPacketId());
            }
            Some(el) => el
        };
        let socket_packet =
            self.socket
                .build_packet_for_payload(data.into(), Event::Message, &self.nsp, Some(id), true)?;

        self.socket.send(socket_packet)?;
        Ok(())
    }

    pub(crate) fn poll(&self) -> Result<Option<Packet>> {
        loop {
            match self.socket.poll() {
                Err(err) => {
                    self.callback(&Event::Error, err.to_string(), None)?;
                    return Err(err);
                }
                Ok(Some(packet)) => {
                    if packet.nsp == self.nsp {
                        self.handle_socketio_packet(&packet)?;
                        return Ok(Some(packet));
                    } else {
                        // Not our namespace continue polling
                    }
                }
                Ok(None) => return Ok(None),
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> Iter {
        Iter { socket: self }
    }

    fn callback<P: Into<Payload>>(&self, event: &Event, payload: P, id: Option<i32>) -> Result<()> {
        let mut on = self.on.lock()?;
        let mut on_any = self.on_any.lock()?;
        let lock = on.deref_mut();
        let on_any_lock = on_any.deref_mut();

        let payload = payload.into();

        if let Some(callback) = lock.get_mut(event) {
            callback(payload.clone(), self.clone(), id);
        }
        match event {
            Event::Message | Event::Custom(_) => {
                if let Some(callback) = on_any_lock {
                    callback(event.clone(), payload, self.clone(), id)
                }
            }
            _ => {}
        }
        drop(on);
        drop(on_any);
        Ok(())
    }

    /// Handles the incoming acks and classifies what callbacks to call and how.
    #[inline]
    fn handle_ack(&self, socket_packet: &Packet) -> Result<()> {
        let Some(id) = socket_packet.id else {
            return Ok(());
        };

        self.outstanding_acks.lock()?.retain_mut(|ack| {
            if ack.id != id {
                return true;
            }

            if ack.time_started.elapsed() < ack.timeout {
                if let Some(ref payload) = socket_packet.data {
                    ack.callback.deref_mut()(Payload::from(payload.to_owned()), self.clone(), socket_packet.id);
                }

                if let Some(ref attachments) = socket_packet.attachments {
                    if let Some(payload) = attachments.get(0) {
                        ack.callback.deref_mut()(Payload::Binary(payload.to_owned()), self.clone(), socket_packet.id);
                    }
                    if ack.time_started.elapsed() < ack.timeout {
                        if let Some(ref payload) = socket_packet.data {
                            ack.callback.deref_mut()(
                                Payload::String(payload.to_owned()),
                                self.clone(),
                                None
                            );
                        }
                        if let Some(ref attachments) = socket_packet.attachments {
                            if let Some(payload) = attachments.get(0) {
                                ack.callback.deref_mut()(
                                    Payload::Binary(payload.to_owned()),
                                    self.clone(),
                                    None
                                );
                            }
                        }
                    } else {
                        // Do something with timed out acks?
                    }
                    }
            } else {
                // Do something with timed out acks?
            }

            false
        });

        Ok(())
    }

    /// Handles a binary event.
    #[inline]
    fn handle_binary_event(&self, packet: &Packet) -> Result<()> {
        let event = if let Some(string_data) = &packet.data {
            string_data.replace('\"', "").into()
        } else {
            Event::Message
        };

        if let Some(attachments) = &packet.attachments {
            if let Some(binary_payload) = attachments.get(0) {
                self.callback(&event, Payload::Binary(binary_payload.to_owned()), packet.id)?;
            }
        }
        Ok(())
    }

    /// A method that parses a packet and eventually calls the corresponding
    /// callback with the supplied data.
    fn handle_event(&self, packet: &Packet) -> Result<()> {
        let Some(ref data) = packet.data else {
            return Ok(());
        };

        // a socketio message always comes in one of the following two flavors (both JSON):
        // 1: `["event", "msg"]`
        // 2: `["msg"]`
        // in case 2, the message is ment for the default message event, in case 1 the event
        // is specified
        if let Ok(Value::Array(contents)) = serde_json::from_str::<Value>(data) {
            let (event, data) = if contents.len() > 1 {
                // case 1
                let event = match contents.first() {
                    Some(Value::String(ev)) => Event::from(ev.as_str()),
                    _ => Event::Message,
                };
                (event, contents.get(1).ok_or(Error::IncompletePacket())?)
            } else {
                // case 2
                (
                    Event::Message,
                    contents.first().ok_or(Error::IncompletePacket())?,
                )
            };

            // call the correct callback
                self.callback(
                    &event,
                    contents
                        .get(1)
                        .unwrap_or_else(|| contents.get(0).unwrap())
                        .to_string(),
                    packet.id
                )?;
        }

        Ok(())
    }

    /// Handles the incoming messages and classifies what callbacks to call and how.
    /// This method is later registered as the callback for the `on_data` event of the
    /// engineio client.
    #[inline]
    fn handle_socketio_packet(&self, packet: &Packet) -> Result<()> {
        if packet.nsp == self.nsp {
            match packet.packet_type {
                PacketId::Ack | PacketId::BinaryAck => {
                    if let Err(err) = self.handle_ack(packet) {
                        self.callback(&Event::Error, err.to_string(), packet.id)?;
                        return Err(err);
                    }
                }
                PacketId::BinaryEvent => {
                    if let Err(err) = self.handle_binary_event(packet) {
                        self.callback(&Event::Error, err.to_string(), packet.id)?;
                    }
                }
                PacketId::Connect => {
                    self.callback(&Event::Connect, "", packet.id)?;
                }
                PacketId::Disconnect => {
                    self.callback(&Event::Close, "", packet.id)?;
                }
                PacketId::ConnectError => {
                    self.callback(
                        &Event::Error,
                        String::from("Received an ConnectError frame: ")
                            + &packet
                                .clone()
                                .data
                                .unwrap_or_else(|| String::from("\"No error message provided\"")),
                        packet.id
                    )?;
                }
                PacketId::Event => {
                    if let Err(err) = self.handle_event(packet) {
                        self.callback(&Event::Error, err.to_string(), packet.id)?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct Iter<'a> {
    socket: &'a RawClient,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<Packet>;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        match self.socket.poll() {
            Err(err) => Some(Err(err)),
            Ok(Some(packet)) => Some(Ok(packet)),
            Ok(None) => None,
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::mpsc;
    use std::thread::sleep;

    use super::*;
    use crate::{client::TransportType, payload::Payload, ClientBuilder};
    use bytes::Bytes;
    use native_tls::TlsConnector;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn socket_io_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        let socket = ClientBuilder::new(url)
            .on("test", |msg, _, _| match msg {
                #[allow(deprecated)]
                Payload::String(str) => println!("Received string: {}", str),
                Payload::Text(text) => println!("Received json: {:#?}", text),
                Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
            })
            .connect()?;

        let payload = json!({"token": 123});
        #[allow(deprecated)]
        let result = socket.emit("test", Payload::String(payload.to_string()));

        assert!(result.is_ok());

        let ack_callback = move |message: Payload, socket: RawClient, _: Option<i32>| {
            let result = socket.emit("test", Payload::Text(vec![json!({"got ack": true})]));
            assert!(result.is_ok());

            println!("Yehaa! My ack got acked?");
            if let Payload::Text(values) = message {
                println!("Received json Ack");
                println!("Ack data: {:#?}", values);
            }
        };

        let ack = socket.emit_with_ack(
            "test",
            Payload::Text(vec![payload]),
            Duration::from_secs(1),
            ack_callback,
        );
        assert!(ack.is_ok());

        sleep(Duration::from_secs(2));

        assert!(socket.disconnect().is_ok());

        Ok(())
    }

    #[test]
    fn socket_io_builder_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        // test socket build logic
        let socket_builder = ClientBuilder::new(url);

        let tls_connector = TlsConnector::builder()
            .use_sni(true)
            .build()
            .expect("Found illegal configuration");

        let socket = socket_builder
            .namespace("/admin")
            .tls_config(tls_connector)
            .opening_header("accept-encoding", "application/json")
            .on("test", |str, _, _| println!("Received: {:#?}", str))
            .on("message", |payload, _, _| println!("{:#?}", payload))
            .connect()?;

        assert!(socket.emit("message", json!("Hello World")).is_ok());

        assert!(socket.emit("binary", Bytes::from_static(&[46, 88])).is_ok());

        assert!(socket
            .emit_with_ack(
                "binary",
                json!("pls ack"),
                Duration::from_secs(1),
                |payload, _, _| {
                    println!("Yehaa the ack got acked");
                    println!("With data: {:#?}", payload);
                }
            )
            .is_ok());

        sleep(Duration::from_secs(2));

        Ok(())
    }

    #[test]
    fn socket_io_builder_integration_iterator() -> Result<()> {
        let url = crate::test::socket_io_server();

        // test socket build logic
        let socket_builder = ClientBuilder::new(url);

        let tls_connector = TlsConnector::builder()
            .use_sni(true)
            .build()
            .expect("Found illegal configuration");

        let socket = socket_builder
            .namespace("/admin")
            .tls_config(tls_connector)
            .opening_header("accept-encoding", "application/json")
            .on("test", |str, _, _| println!("Received: {:#?}", str))
            .on("message", |payload, _, _| println!("{:#?}", payload))
            .connect_raw()?;

        assert!(socket.emit("message", json!("Hello World")).is_ok());

        assert!(socket.emit("binary", Bytes::from_static(&[46, 88])).is_ok());

        assert!(socket
            .emit_with_ack(
                "binary",
                json!("pls ack"),
                Duration::from_secs(1),
                |payload, _, _| {
                    println!("Yehaa the ack got acked");
                    println!("With data: {:#?}", payload);
                }
            )
            .is_ok());

        test_socketio_socket(socket, "/admin".to_owned())
    }

    #[test]
    fn socket_io_on_any_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        let (tx, rx) = mpsc::sync_channel(1);

        let _socket = ClientBuilder::new(url)
            .namespace("/")
            .auth(json!({ "password": "123" }))
            .on("auth", |payload, _client, _| {
                if let Payload::Text(payload) = payload {
                    println!("{:#?}", payload);
                }
            })
            .on_any(move |event, payload, _client, _| {
                if let Payload::Text(payload) = payload {
                    println!("{event} {payload:#?}");
                }
                tx.send(String::from(event)).unwrap();
            })
            .connect()?;

        // Sleep to give server enough time to send 2 events
        sleep(Duration::from_secs(2));

        let event = rx.recv().unwrap();
        assert_eq!(event, "message");
        let event = rx.recv().unwrap();
        assert_eq!(event, "test");

        Ok(())
    }

    #[test]
    fn socket_io_auth_builder_integration() -> Result<()> {
        let url = crate::test::socket_io_auth_server();
        let nsp = String::from("/admin");
        let socket = ClientBuilder::new(url)
            .namespace(nsp.clone())
            .auth(json!({ "password": "123" }))
            .connect_raw()?;

        let mut iter = socket
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_type != PacketId::Connect);

        let packet: Option<Packet> = iter.next();
        assert!(packet.is_some());

        let packet = packet.unwrap();

        assert_eq!(
            packet,
            Packet::new(
                PacketId::Event,
                nsp,
                Some("[\"auth\",\"success\"]".to_owned()),
                None,
                0,
                None
            )
        );

        Ok(())
    }

    #[test]
    fn socketio_polling_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::Polling)
            .connect_raw()?;
        test_socketio_socket(socket, "/".to_owned())
    }

    #[test]
    fn socket_io_websocket_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::Websocket)
            .connect_raw()?;
        test_socketio_socket(socket, "/".to_owned())
    }

    #[test]
    fn socket_io_websocket_upgrade_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::WebsocketUpgrade)
            .connect_raw()?;
        test_socketio_socket(socket, "/".to_owned())
    }

    #[test]
    fn socket_io_any_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::Any)
            .connect_raw()?;
        test_socketio_socket(socket, "/".to_owned())
    }

    fn test_socketio_socket(socket: RawClient, nsp: String) -> Result<()> {
        let mut iter = socket
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_type != PacketId::Connect);

        let packet: Option<Packet> = iter.next();
        assert!(packet.is_some());

        let packet = packet.unwrap();

        assert_eq!(
            packet,
            Packet::new(
                PacketId::Event,
                nsp.clone(),
                Some("[\"Hello from the message event!\"]".to_owned()),
                None,
                0,
                None,
            )
        );

        let packet: Option<Packet> = iter.next();
        assert!(packet.is_some());

        let packet = packet.unwrap();

        assert_eq!(
            packet,
            Packet::new(
                PacketId::Event,
                nsp.clone(),
                Some("[\"test\",\"Hello from the test event!\"]".to_owned()),
                None,
                0,
                None
            )
        );

        let packet: Option<Packet> = iter.next();
        assert!(packet.is_some());

        let packet = packet.unwrap();
        assert_eq!(
            packet,
            Packet::new(
                PacketId::BinaryEvent,
                nsp.clone(),
                None,
                None,
                1,
                Some(vec![Bytes::from_static(&[4, 5, 6])]),
            )
        );

        let packet: Option<Packet> = iter.next();
        assert!(packet.is_some());

        let packet = packet.unwrap();
        assert_eq!(
            packet,
            Packet::new(
                PacketId::BinaryEvent,
                nsp.clone(),
                Some("\"test\"".to_owned()),
                None,
                1,
                Some(vec![Bytes::from_static(&[1, 2, 3])]),
            )
        );

        let packet: Option<Packet> = iter.next();

        assert!(packet.is_some());

        let packet = packet.unwrap();
        assert_eq!(
            packet,
            Packet::new(
                PacketId::Event,
                nsp.clone(),
                Some(
                    serde_json::Value::Array(vec![
                        serde_json::Value::from("This is the first argument"),
                        serde_json::Value::from("This is the second argument"),
                        serde_json::json!({"argCount":3})
                    ])
                    .to_string()
                ),
                None,
                0,
                None,
            )
        );

        assert!(socket
            .emit_with_ack(
                "test",
                Payload::from("123"),
                Duration::from_secs(10),
                |message: Payload, _, _| {
                    println!("Yehaa! My ack got acked?");
                    if let Payload::Text(values) = message {
                        println!("Received ack");
                        println!("Ack data: {values:#?}");
                    }
                }
            )
            .is_ok());

        Ok(())
    }

    // TODO: add secure socketio server
}
