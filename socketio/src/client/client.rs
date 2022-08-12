pub use super::super::{event::Event, payload::Payload};
use super::callback::Callback;
use crate::packet::{Packet, PacketId};
use crate::Error;
use rand::{thread_rng, Rng};

use crate::client::callback::{SocketAnyCallback, SocketCallback};
use crate::error::Result;
use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use std::time::Instant;

use crate::socket::Socket as InnerSocket;

type BuildSocketFn = dyn Fn() -> Result<InnerSocket> + Send + Sync;

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
pub struct Client {
    inner: Arc<RwLock<Inner>>,
}

#[derive(Clone)]
struct Inner {
    /// The inner socket client to delegate the methods to.
    socket: Option<Arc<RwLock<InnerSocket>>>,
    socket_fn: Arc<Mutex<Box<BuildSocketFn>>>,
    on: Arc<RwLock<HashMap<Event, Callback<SocketCallback>>>>,
    on_any: Arc<RwLock<Option<Callback<SocketAnyCallback>>>>,
    outstanding_acks: Arc<RwLock<Vec<Ack>>>,
    // namespace, for multiplexing messages
    nsp: String,
    // Data sent in opening header
    auth: Option<serde_json::Value>,
    backoff: ExponentialBackoff,
}

impl Client {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new<T: Into<String>>(
        socket_fn: Box<BuildSocketFn>,
        namespace: T,
        on: HashMap<Event, Callback<SocketCallback>>,
        on_any: Option<Callback<SocketAnyCallback>>,
        auth: Option<serde_json::Value>,
    ) -> Result<Self> {
        Ok(Client {
            inner: Arc::new(RwLock::new(Inner {
                socket: None,
                socket_fn: Arc::new(Mutex::new(socket_fn)),
                nsp: namespace.into(),
                on: Arc::new(RwLock::new(on)),
                on_any: Arc::new(RwLock::new(on_any)),
                outstanding_acks: Arc::new(RwLock::new(Vec::new())),
                auth,
                backoff: ExponentialBackoff::default(),
            })),
        })
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server. Attention: it's not allowed to add a
    /// callback after a call to this method.
    pub(crate) fn connect(&self) -> Result<()> {
        let mut inner = self.inner.write()?;

        let socket_fn = inner.socket_fn.lock()?;
        let socket = socket_fn()?;
        drop(socket_fn);

        // Connect the underlying socket
        socket.connect()?;

        let auth = inner.auth.as_ref().map(|data| data.to_string());

        // construct the opening packet
        let open_packet = Packet::new(PacketId::Connect, inner.nsp.clone(), auth, None, 0, None);

        socket.send(open_packet)?;

        inner.socket = Some(Arc::new(RwLock::new(socket)));

        self.poll_callback();

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
    /// use rust_socketio::{ClientBuilder, Client, Payload};
    /// use serde_json::json;
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("test", |payload: Payload, socket: Client| {
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
        let inner = self.inner.read()?;
        let socket = inner
            .socket
            .clone()
            .ok_or(Error::IllegalActionBeforeOpen())?;
        let socket = socket.read()?;

        socket.emit(&inner.nsp, event.into(), data.into())
    }

    /// Disconnects this client from the server by sending a `socket.io` closing
    /// packet.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload, Client};
    /// use serde_json::json;
    ///
    /// fn handle_test(payload: Payload, socket: Client) {
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
        let inner = self.inner.read()?;
        let disconnect_packet =
            Packet::new(PacketId::Disconnect, inner.nsp.clone(), None, None, 0, None);

        let socket = inner
            .socket
            .clone()
            .ok_or(Error::IllegalActionBeforeOpen())?;
        let socket = socket.read()?;

        socket.send(disconnect_packet)?;
        socket.disconnect()?;

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
    /// use rust_socketio::{ClientBuilder, Payload, Client};
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use std::thread::sleep;
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("foo", |payload: Payload, _| println!("Received: {:#?}", payload))
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// let ack_callback = |message: Payload, socket: Client| {
    ///     match message {
    ///         Payload::String(str) => println!("{}", str),
    ///         Payload::Binary(bytes) => println!("Received bytes: {:#?}", bytes),
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
        F: for<'a> FnMut(Payload, Client) + 'static + Sync + Send,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let inner = self.inner.read()?;
        let socket = inner
            .socket
            .clone()
            .ok_or(Error::IllegalActionBeforeOpen())?;
        let socket = socket.read()?;

        let id = thread_rng().gen_range(0..999);
        let socket_packet =
            socket.build_packet_for_payload(data.into(), event.into(), &inner.nsp, Some(id))?;

        let ack = Ack {
            id,
            time_started: Instant::now(),
            timeout,
            callback: Callback::<SocketCallback>::new(callback),
        };

        // add the ack to the tuple of outstanding acks
        inner.outstanding_acks.write()?.push(ack);

        socket.send(socket_packet)?;

        Ok(())
    }

    pub(crate) fn poll(&self) -> Result<Option<Packet>> {
        loop {
            let inner = self.inner.read()?;
            let socket = inner
                .socket
                .clone()
                .ok_or(Error::IllegalActionBeforeOpen())?;
            let socket = socket.read()?;

            match socket.poll() {
                Err(err) => {
                    self.callback(&Event::Error, err.to_string())?;
                    return Err(err);
                }
                Ok(Some(packet)) => {
                    if packet.nsp == inner.nsp {
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

    pub(crate) fn iter(&self) -> Iter {
        Iter { socket: self }
    }

    fn callback<P: Into<Payload>>(&self, event: &Event, payload: P) -> Result<()> {
        let inner = self.inner.read()?;
        let mut on = inner.on.write()?;
        let mut on_any = inner.on_any.write()?;
        let lock = on.deref_mut();
        let on_any_lock = on_any.deref_mut();

        let payload = payload.into();

        if let Some(callback) = lock.get_mut(event) {
            callback(payload.clone(), self.clone());
        }
        match event {
            Event::Message | Event::Custom(_) => {
                if let Some(callback) = on_any_lock {
                    callback(event.clone(), payload, self.clone())
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
        let mut to_be_removed = Vec::new();
        let inner = self.inner.read()?;
        if let Some(id) = socket_packet.id {
            for (index, ack) in inner.outstanding_acks.write()?.iter_mut().enumerate() {
                if ack.id == id {
                    to_be_removed.push(index);

                    if ack.time_started.elapsed() < ack.timeout {
                        if let Some(ref payload) = socket_packet.data {
                            ack.callback.deref_mut()(
                                Payload::String(payload.to_owned()),
                                self.clone(),
                            );
                        }
                        if let Some(ref attachments) = socket_packet.attachments {
                            if let Some(payload) = attachments.get(0) {
                                ack.callback.deref_mut()(
                                    Payload::Binary(payload.to_owned()),
                                    self.clone(),
                                );
                            }
                        }
                    } else {
                        // Do something with timed out acks?
                    }
                }
            }
            for index in to_be_removed {
                inner.outstanding_acks.write()?.remove(index);
            }
        }
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
                self.callback(&event, Payload::Binary(binary_payload.to_owned()))?;
            }
        }
        Ok(())
    }

    /// A method for handling the Event Client Packets.
    // this could only be called with an event
    fn handle_event(&self, packet: &Packet) -> Result<()> {
        // unwrap the potential data
        if let Some(data) = &packet.data {
            // the string must be a valid json array with the event at index 0 and the
            // payload at index 1. if no event is specified, the message callback is used
            if let Ok(serde_json::Value::Array(contents)) =
                serde_json::from_str::<serde_json::Value>(data)
            {
                let event: Event = if contents.len() > 1 {
                    contents
                        .get(0)
                        .map(|value| match value {
                            serde_json::Value::String(ev) => ev,
                            _ => "message",
                        })
                        .unwrap_or("message")
                        .into()
                } else {
                    Event::Message
                };
                self.callback(
                    &event,
                    contents
                        .get(1)
                        .unwrap_or_else(|| contents.get(0).unwrap())
                        .to_string(),
                )?;
            }
        }
        Ok(())
    }

    /// Handles the incoming messages and classifies what callbacks to call and how.
    /// This method is later registered as the callback for the `on_data` event of the
    /// engineio client.
    #[inline]
    fn handle_socketio_packet(&self, packet: &Packet) -> Result<()> {
        let inner = self.inner.read()?;
        if packet.nsp == inner.nsp {
            match packet.packet_type {
                PacketId::Ack | PacketId::BinaryAck => {
                    if let Err(err) = self.handle_ack(packet) {
                        self.callback(&Event::Error, err.to_string())?;
                        return Err(err);
                    }
                }
                PacketId::BinaryEvent => {
                    if let Err(err) = self.handle_binary_event(packet) {
                        self.callback(&Event::Error, err.to_string())?;
                    }
                }
                PacketId::Connect => {
                    self.callback(&Event::Connect, "")?;
                }
                PacketId::Disconnect => {
                    self.callback(&Event::Close, "")?;
                }
                PacketId::ConnectError => {
                    self.callback(
                        &Event::Error,
                        String::from("Received an ConnectError frame: ")
                            + &packet
                                .clone()
                                .data
                                .unwrap_or_else(|| String::from("\"No error message provided\"")),
                    )?;
                }
                PacketId::Event => {
                    if let Err(err) = self.handle_event(packet) {
                        self.callback(&Event::Error, err.to_string())?;
                    }
                }
            }
        }
        Ok(())
    }

    fn poll_callback(&self) {
        let self_clone = self.clone();
        // Use thread to consume items in iterator in order to call callbacks
        std::thread::spawn(move || {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            for packet in self_clone.iter() {
                if let e @ Err(Error::IncompleteResponseFromEngineIo(_)) = packet {
                    //TODO: 0.3.X handle errors
                    self_clone.reconnect();
                    panic!("{}", e.unwrap_err());
                }
            }
        });
    }

    fn reconnect(&self) {
        let self_clone = self.clone();
        std::thread::spawn(move || {
            let _ = self_clone.disconnect();
            loop {
                match self_clone.connect() {
                    Ok(_) => break,
                    Err(_) => {
                        let backoff = self_clone.next_backoff();
                        if let Some(d) = backoff {
                            std::thread::sleep(d);
                        }
                    }
                }
            }
        });
    }

    fn next_backoff(&self) -> Option<Duration> {
        let mut inner = self.inner.write().ok()?;
        inner.backoff.next_backoff()
    }
}

pub struct Iter<'a> {
    socket: &'a Client,
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
    use std::sync::mpsc::Receiver;
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
            .on("test", |msg, _| match msg {
                Payload::String(str) => println!("Received string: {}", str),
                Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
            })
            .connect()?;

        let payload = json!({"token": 123});
        let result = socket.emit("test", Payload::String(payload.to_string()));

        assert!(result.is_ok());

        let ack_callback = move |message: Payload, socket: Client| {
            let result = socket.emit(
                "test",
                Payload::String(json!({"got ack": true}).to_string()),
            );
            assert!(result.is_ok());

            println!("Yehaa! My ack got acked?");
            if let Payload::String(str) = message {
                println!("Received string Ack");
                println!("Ack data: {}", str);
            }
        };

        let ack = socket.emit_with_ack(
            "test",
            Payload::String(payload.to_string()),
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
            .on("test", |str, _| println!("Received: {:#?}", str))
            .on("message", |payload, _| println!("{:#?}", payload))
            .connect()?;

        assert!(socket.emit("message", json!("Hello World")).is_ok());

        assert!(socket.emit("binary", Bytes::from_static(&[46, 88])).is_ok());

        assert!(socket
            .emit_with_ack(
                "binary",
                json!("pls ack"),
                Duration::from_secs(1),
                |payload, _| {
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

        let (tx, rx) = mpsc::sync_channel(1);

        let socket = socket_builder
            .namespace("/admin")
            .tls_config(tls_connector)
            .opening_header("accept-encoding", "application/json")
            .on("test", |str, _| println!("Received: {:#?}", str))
            .on("message", |payload, _| println!("{:#?}", payload))
            .on_any(move |event, payload, _client| {
                tx.send((String::from(event), payload)).unwrap();
            })
            .connect_manual()?;

        assert!(socket.emit("message", json!("Hello World")).is_ok());

        assert!(socket.emit("binary", Bytes::from_static(&[46, 88])).is_ok());

        assert!(socket
            .emit_with_ack(
                "binary",
                json!("pls ack"),
                Duration::from_secs(1),
                |payload, _| {
                    println!("Yehaa the ack got acked");
                    println!("With data: {:#?}", payload);
                }
            )
            .is_ok());

        test_socketio_socket(socket, "/admin".to_owned(), rx)
    }

    #[test]
    fn socket_io_on_any_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let (tx, rx) = mpsc::sync_channel(1);
        let _socket = ClientBuilder::new(url)
            .namespace("/")
            .auth(json!({ "password": "123" }))
            .on("auth", |payload, _client| {
                if let Payload::String(msg) = payload {
                    println!("{}", msg);
                }
            })
            .on_any(move |event, payload, _client| {
                if let Payload::String(str) = payload {
                    println!("{} {}", String::from(event.clone()), str);
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
        let (tx, rx) = mpsc::sync_channel(1);
        let _socket = ClientBuilder::new(url)
            .namespace(nsp.clone())
            .auth(json!({ "password": "123" }))
            .on_any(move |event, payload, _client| {
                tx.send((String::from(event), payload)).unwrap();
            })
            .connect_manual()?;

        let (event, payload) = rx.recv().unwrap();
        assert_eq!(event, "auth".to_owned());
        match payload {
            Payload::Binary(_) => assert!(false),
            Payload::String(p) => assert_eq!(p, "\"success\"".to_owned()),
        };

        Ok(())
    }

    #[test]
    fn socketio_polling_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let (tx, rx) = mpsc::sync_channel(1);
        let socket = ClientBuilder::new(url.clone())
            .transport_type(TransportType::Polling)
            .on_any(move |event, payload, _client| {
                tx.send((String::from(event), payload)).unwrap();
            })
            .connect_manual()?;
        test_socketio_socket(socket, "/".to_owned(), rx)
    }

    #[test]
    fn socket_io_websocket_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let (tx, rx) = mpsc::sync_channel(1);
        let socket = ClientBuilder::new(url.clone())
            .transport_type(TransportType::Websocket)
            .on_any(move |event, payload, _client| {
                tx.send((String::from(event), payload)).unwrap();
            })
            .connect_manual()?;
        test_socketio_socket(socket, "/".to_owned(), rx)
    }

    #[test]
    fn socket_io_websocket_upgrade_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let (tx, rx) = mpsc::sync_channel(1);

        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::WebsocketUpgrade)
            .on_any(move |event, payload, _client| {
                tx.send((String::from(event), payload)).unwrap();
            })
            .connect_manual()?;
        test_socketio_socket(socket, "/".to_owned(), rx)
    }

    #[test]
    fn socket_io_any_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let (tx, rx) = mpsc::sync_channel(1);
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::Any)
            .on_any(move |event, payload, _client| {
                tx.send((String::from(event), payload)).unwrap();
            })
            .connect_manual()?;
        test_socketio_socket(socket, "/".to_owned(), rx)
    }

    fn test_socketio_socket(
        socket: Client,
        _nsp: String,
        rx: Receiver<(String, Payload)>,
    ) -> Result<()> {
        let _iter = socket
            .iter()
            .map(|packet| packet.unwrap())
            .filter(|packet| packet.packet_type != PacketId::Connect);

        let (event, payload) = rx.recv().unwrap();
        assert_eq!(event, "message".to_owned());
        match payload {
            Payload::Binary(_) => assert!(false),
            Payload::String(p) => assert_eq!(p, "\"Hello from the message event!\"".to_owned()),
        };

        let (event, payload) = rx.recv().unwrap();
        assert_eq!(event, "test".to_owned());
        match payload {
            Payload::Binary(_) => assert!(false),
            Payload::String(p) => assert_eq!(p, "\"Hello from the test event!\"".to_owned()),
        };

        let (event, payload) = rx.recv().unwrap();
        assert_eq!(event, "message".to_owned());
        match payload {
            Payload::Binary(b) => assert_eq!(b, Bytes::from_static(&[4, 5, 6])),
            Payload::String(_) => assert!(false),
        };

        let (event, payload) = rx.recv().unwrap();
        assert_eq!(event, "test".to_owned());
        match payload {
            Payload::Binary(b) => assert_eq!(b, Bytes::from_static(&[1, 2, 3])),
            Payload::String(_) => assert!(false),
        };

        assert!(socket
            .emit_with_ack(
                "test",
                Payload::String("123".to_owned()),
                Duration::from_secs(10),
                |message: Payload, _| {
                    println!("Yehaa! My ack got acked?");
                    if let Payload::String(str) = message {
                        println!("Received string ack");
                        println!("Ack data: {}", str);
                    }
                }
            )
            .is_ok());

        Ok(())
    }

    // TODO: 0.3.X add secure socketio server
}
