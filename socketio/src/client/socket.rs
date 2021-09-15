pub use super::super::{event::Event, payload::Payload};
use super::callback::Callback;
use crate::packet::{Packet, PacketId};
use rand::{thread_rng, Rng};

use crate::error::Result;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::{Arc, RwLock};
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
    callback: Callback,
}

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the server will connect to the default namespace `"/"`.
#[derive(Clone)]
pub struct Socket {
    /// The inner socket client to delegate the methods to.
    socket: InnerSocket,
    on: Arc<RwLock<HashMap<Event, Callback>>>,
    outstanding_acks: Arc<RwLock<Vec<Ack>>>,
    // namespace, for multiplexing messages
    nsp: String,
}

impl Socket {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new<T: Into<String>>(
        socket: InnerSocket,
        namespace: T,
        on: HashMap<Event, Callback>,
    ) -> Result<Self> {
        Ok(Socket {
            socket,
            nsp: namespace.into(),
            on: Arc::new(RwLock::new(on)),
            outstanding_acks: Arc::new(RwLock::new(Vec::new())),
        })
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server. Attention: it's not allowed to add a
    /// callback after a call to this method.
    pub(crate) fn connect(&self) -> Result<()> {
        // Connect the underlying socket
        self.socket.connect()?;

        // construct the opening packet
        let open_packet = Packet::new(PacketId::Connect, self.nsp.clone(), None, None, 0, None);

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
    /// use rust_socketio::{SocketBuilder, Socket, Payload};
    /// use serde_json::json;
    ///
    /// let mut socket = SocketBuilder::new("http://localhost:4200/")
    ///     .on("test", |payload: Payload, socket: Socket| {
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
    /// use rust_socketio::{SocketBuilder, Payload, Socket};
    /// use serde_json::json;
    ///
    /// fn handle_test(payload: Payload, socket: Socket) {
    ///     println!("Received: {:#?}", payload);
    ///     socket.emit("test", json!({"hello": true})).expect("Server unreachable");
    /// }
    ///
    /// let mut socket = SocketBuilder::new("http://localhost:4200/")
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

        self.socket.send(disconnect_packet)?;
        self.socket.disconnect()?;

        Ok(())
    }

    /// Sends a message to the server but `alloc`s an `ack` to check whether the
    /// server responded in a given timespan. This message takes an event, which
    /// could either be one of the common events like "message" or "error" or a
    /// custom event like "foo", as well as a data parameter. But be careful,
    /// in case you send a [`Payload::String`], the string needs to be valid JSON.
    /// It's even recommended to use a library like serde_json to serialize the data properly.
    /// It also requires a timeout `Duration` in which the client needs to answer.
    /// If the ack is acked in the correct timespan, the specified callback is
    /// called. The callback consumes a [`Payload`] which represents the data send
    /// by the server.
    ///
    /// Stored closures MUST annotate &Socket due to a quirk with Rust's lifetime inference.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::{SocketBuilder, Payload, Socket};
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use std::thread::sleep;
    ///
    /// let mut socket = SocketBuilder::new("http://localhost:4200/")
    ///     .on("foo", |payload: Payload, _| println!("Received: {:#?}", payload))
    ///     .connect()
    ///     .expect("connection failed");
    ///
    ///
    /// // &Socket MUST be annotated when storing a closure in a variable before
    /// // passing to emit_with_awk. Inline closures and calling functions work as
    /// // intended.
    /// let ack_callback = |message: Payload, socket: Socket| {
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
        F: for<'a> FnMut(Payload, Socket) + 'static + Sync + Send,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let id = thread_rng().gen_range(0..999);
        let socket_packet =
            self.socket
                .build_packet_for_payload(data.into(), event.into(), &self.nsp, Some(id))?;

        let ack = Ack {
            id,
            time_started: Instant::now(),
            timeout,
            callback: Callback::new(callback),
        };

        // add the ack to the tuple of outstanding acks
        self.outstanding_acks.write()?.push(ack);

        self.socket.send(socket_packet)?;
        Ok(())
    }

    pub(crate) fn poll(&self) -> Result<Option<Packet>> {
        loop {
            match self.socket.poll() {
                Err(err) => {
                    self.callback(&Event::Error, err.to_string())?;
                    return Err(err.into());
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

    pub fn iter(&self) -> Iter {
        Iter { socket: self }
    }

    fn callback<P: Into<Payload>>(&self, event: &Event, payload: P) -> Result<()> {
        let mut on = self.on.write()?;
        let lock = on.deref_mut();
        if let Some(callback) = lock.get_mut(event) {
            callback(payload.into(), self.clone());
        }
        drop(lock);
        drop(on);
        Ok(())
    }

    /// Handles the incoming acks and classifies what callbacks to call and how.
    #[inline]
    fn handle_ack(&self, socket_packet: &Packet) -> Result<()> {
        let mut to_be_removed = Vec::new();
        if let Some(id) = socket_packet.id {
            for (index, ack) in self.outstanding_acks.write()?.iter_mut().enumerate() {
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
                self.outstanding_acks.write()?.remove(index);
            }
        }
        Ok(())
    }

    /// Handles a binary event.
    #[inline]
    fn handle_binary_event(&self, packet: &Packet) -> Result<()> {
        let event = if let Some(string_data) = &packet.data {
            string_data.replace("\"", "").into()
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

    /// A method for handling the Event Socket Packets.
    // this could only be called with an event
    fn handle_event(&self, packet: &Packet) -> Result<()> {
        // unwrap the potential data
        if let Some(data) = &packet.data {
            // the string must be a valid json array with the event at index 0 and the
            // payload at index 1. if no event is specified, the message callback is used
            if let Ok(serde_json::Value::Array(contents)) =
                serde_json::from_str::<serde_json::Value>(&data)
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
        if packet.nsp == self.nsp {
            match packet.packet_type {
                PacketId::Ack | PacketId::BinaryAck => match self.handle_ack(&packet) {
                    Err(err) => {
                        self.callback(&Event::Error, err.to_string())?;
                        return Err(err.into());
                    }
                    Ok(_) => (),
                },
                PacketId::BinaryEvent => {
                    if let Err(err) = self.handle_binary_event(&packet) {
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
                    if let Err(err) = self.handle_event(&packet) {
                        self.callback(&Event::Error, err.to_string())?;
                    }
                }
            }
        }
        Ok(())
    }
}

pub struct Iter<'a> {
    socket: &'a Socket,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<Packet>;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        match self.socket.poll() {
            Err(err) => return Some(Err(err)),
            Ok(Some(packet)) => return Some(Ok(packet)),
            Ok(None) => return None,
        }
    }
}

#[cfg(test)]
mod test {

    use std::thread::sleep;

    use super::*;
    use crate::{client::TransportType, payload::Payload, SocketBuilder};
    use bytes::Bytes;
    use native_tls::TlsConnector;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn socket_io_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        let socket = SocketBuilder::new(url)
            .on("test", |msg, _| match msg {
                Payload::String(str) => println!("Received string: {}", str),
                Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
            })
            .connect()?;

        let payload = json!({"token": 123});
        let result = socket.emit("test", Payload::String(payload.to_string()));

        assert!(result.is_ok());

        let ack_callback = move |message: Payload, socket: Socket| {
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
            Duration::from_secs(2),
            ack_callback,
        );
        assert!(ack.is_ok());

        sleep(Duration::from_secs(3));

        assert!(socket.disconnect().is_ok());

        Ok(())
    }

    #[test]
    fn socket_io_builder_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        // test socket build logic
        let socket_builder = SocketBuilder::new(url.clone());

        let tls_connector = TlsConnector::builder()
            .use_sni(true)
            .build()
            .expect("Found illegal configuration");

        let socket = socket_builder
            .namespace("/")
            .tls_config(tls_connector)
            .opening_header("host", "localhost")
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

        sleep(Duration::from_secs(5));

        let socket = SocketBuilder::new(url.clone())
            .transport_type(TransportType::Polling)
            .connect()?;
        test_socketio_socket(socket)?;
        let socket = SocketBuilder::new(url.clone())
            .transport_type(TransportType::Websocket)
            .connect()?;
        test_socketio_socket(socket)?;
        let socket = SocketBuilder::new(url.clone())
            .transport_type(TransportType::WebsocketUpgrade)
            .connect()?;
        test_socketio_socket(socket)?;
        let socket = SocketBuilder::new(url.clone())
            .transport_type(TransportType::Any)
            .connect()?;
        test_socketio_socket(socket)?;

        Ok(())
    }

    fn test_socketio_socket(socket: Socket) -> Result<()> {
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
                "/".to_owned(),
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
                "/".to_owned(),
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
                "/".to_owned(),
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
                "/".to_owned(),
                Some("\"test\"".to_owned()),
                None,
                1,
                Some(vec![Bytes::from_static(&[1, 2, 3])]),
            )
        );

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

    fn builder() -> SocketBuilder {
        let url = crate::test::socket_io_server();
        SocketBuilder::new(url)
            .on("test", |message, _| {
                if let Payload::String(st) = message {
                    println!("{}", st)
                }
            })
            .on("Error", |_, _| {})
            .on("Connect", |_, _| {})
            .on("Close", |_, _| {})
    }

    #[test]
    fn test_connection() -> Result<()> {
        let socket = builder()
            .transport_type(TransportType::Any)
            .connect_manual()?;

        test_socketio_socket(socket)
    }

    #[test]
    fn test_connection_polling() -> Result<()> {
        let socket = builder()
            .transport_type(TransportType::Polling)
            .connect_manual()?;

        test_socketio_socket(socket)
    }

    #[test]
    fn test_connection_websocket() -> Result<()> {
        let socket = builder()
            .transport_type(TransportType::Websocket)
            .connect_manual()?;

        test_socketio_socket(socket)
    }

    #[test]
    fn test_connection_websocket_upgrade() -> Result<()> {
        let socket = builder()
            .transport_type(TransportType::WebsocketUpgrade)
            .connect_manual()?;

        test_socketio_socket(socket)
    }

    // TODO: 0.3.X add secure socketio server
}
