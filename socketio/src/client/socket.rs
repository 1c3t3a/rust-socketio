pub use super::super::{event::Event, payload::Payload};
use crate::packet::Packet;
use native_tls::TlsConnector;
use rust_engineio::{
    header::{HeaderMap, HeaderValue},
    Socket as EngineSocket, SocketBuilder as EngineSocketBuilder,
};
use url::Url;

use crate::error::Result;
use std::{time::Duration, vec};

use crate::socket::Socket as InnerSocket;

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the server will connect to the default namespace `"/"`.
#[derive(Debug, Clone)]
pub struct Socket {
    /// The inner socket client to delegate the methods to.
    // TODO: Make this private
    pub socket: InnerSocket<EngineSocket>,
}

type SocketCallback = dyn FnMut(Payload, Socket) + 'static + Sync + Send;

/// Flavor of engineio transport
//TODO: Consider longevity
#[derive(Clone, Eq, PartialEq)]
pub enum TransportType {
    Any,
    Websocket,
    WebsocketUpgrade,
    Polling,
}

/// A builder class for a `socket.io` socket. This handles setting up the client and
/// configuring the callback, the namespace and metadata of the socket. If no
/// namespace is specified, the default namespace `/` is taken. The `connect` method
/// acts the `build` method and returns a connected [`Socket`].
pub struct SocketBuilder {
    address: String,
    on: Option<Vec<(Event, Box<SocketCallback>)>>,
    namespace: Option<String>,
    tls_config: Option<TlsConnector>,
    opening_headers: Option<HeaderMap>,
    transport_type: TransportType,
}

impl SocketBuilder {
    /// Create as client builder from a URL. URLs must be in the form
    /// `[ws or wss or http or https]://[domain]:[port]/[path]`. The
    /// path of the URL is optional and if no port is given, port 80
    /// will be used.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload};
    /// use serde_json::json;
    ///
    ///
    /// let callback = |payload: Payload, _| {
    ///            match payload {
    ///                Payload::String(str) => println!("Received: {}", str),
    ///                Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///            }
    /// };
    ///
    /// let mut socket = SocketBuilder::new("http://localhost:4200")
    ///     .namespace("/admin")
    ///     .on("test", callback)
    ///     .connect()
    ///     .expect("error while connecting");
    ///
    /// // use the socket
    /// let json_payload = json!({"token": 123});
    ///
    /// let result = socket.emit("foo", json_payload);
    ///
    /// assert!(result.is_ok());
    /// ```
    pub fn new<T: Into<String>>(address: T) -> Self {
        Self {
            address: address.into(),
            on: None,
            namespace: None,
            tls_config: None,
            opening_headers: None,
            transport_type: TransportType::Any,
        }
    }

    /// Sets the target namespace of the client. The namespace should start
    /// with a leading `/`. Valid examples are e.g. `/admin`, `/foo`.
    pub fn namespace<T: Into<String>>(mut self, namespace: T) -> Self {
        let mut nsp = namespace.into();
        if !nsp.starts_with('/') {
            nsp = "/".to_owned() + &nsp;
        }
        self.namespace = Some(nsp);
        self
    }

    /// Registers a new callback for a certain [`socketio::event::Event`]. The event could either be
    /// one of the common events like `message`, `error`, `connect`, `close` or a custom
    /// event defined by a string, e.g. `onPayment` or `foo`.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload};
    ///
    /// let callback = |payload: Payload, _| {
    ///            match payload {
    ///                Payload::String(str) => println!("Received: {}", str),
    ///                Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///            }
    /// };
    ///
    /// let socket = SocketBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on("test", callback)
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .connect();
    ///
    /// ```
    pub fn on<F>(mut self, event: &str, callback: F) -> Self
    where
        F: FnMut(Payload, Socket) + 'static + Sync + Send,
    {
        match self.on {
            Some(ref mut vector) => vector.push((event.into(), Box::new(callback))),
            None => self.on = Some(vec![(event.into(), Box::new(callback))]),
        }
        self
    }

    /// Uses a preconfigured TLS connector for secure cummunication. This configures
    /// both the `polling` as well as the `websocket` transport type.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload};
    /// use native_tls::TlsConnector;
    ///
    /// let tls_connector =  TlsConnector::builder()
    ///            .use_sni(true)
    ///            .build()
    ///            .expect("Found illegal configuration");
    ///
    /// let socket = SocketBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .tls_config(tls_connector)
    ///     .connect();
    ///
    /// ```
    pub fn tls_config(mut self, tls_config: TlsConnector) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Sets custom http headers for the opening request. The headers will be passed to the underlying
    /// transport type (either websockets or polling) and then get passed with every request thats made.
    /// via the transport layer.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload};
    ///
    ///
    /// let socket = SocketBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .opening_header("accept-encoding", "application/json")
    ///     .connect();
    ///
    /// ```
    pub fn opening_header<T: Into<HeaderValue>, K: Into<String>>(mut self, key: K, val: T) -> Self {
        match self.opening_headers {
            Some(ref mut map) => {
                map.insert(key.into(), val.into());
            }
            None => {
                let mut map = HeaderMap::new();
                map.insert(key.into(), val.into());
                self.opening_headers = Some(map);
            }
        }
        self
    }

    /// Specifies which underlying transport type to use
    // TODO: better docs
    pub fn transport_type(mut self, transport_type: TransportType) -> Self {
        self.transport_type = transport_type;

        self
    }

    /// Connects the socket to a certain endpoint. This returns a connected
    /// [`Socket`] instance. This method returns an [`std::result::Result::Err`]
    /// value if something goes wrong during connection.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload};
    /// use serde_json::json;
    ///
    ///
    /// let mut socket = SocketBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on("error", |err, _| eprintln!("Socket error!: {:#?}", err))
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// // use the socket
    /// let json_payload = json!({"token": 123});
    ///
    /// let result = socket.emit("foo", json_payload);
    ///
    /// assert!(result.is_ok());
    /// ```
    pub fn connect(self) -> Result<Socket> {
        let mut url = Url::parse(&self.address)?;

        if url.path() == "/" {
            url.set_path("/socket.io/");
        }

        let mut builder = EngineSocketBuilder::new(url);

        if let Some(tls_config) = self.tls_config {
            builder = builder.tls_config(tls_config);
        }
        if let Some(headers) = self.opening_headers {
            builder = builder.headers(headers);
        }

        let socket = match self.transport_type {
            TransportType::Any => builder.build_with_fallback()?,
            TransportType::Polling => builder.build_polling()?,
            TransportType::Websocket => builder.build_websocket()?,
            TransportType::WebsocketUpgrade => builder.build_websocket_with_upgrade()?,
        };

        let mut socket = Socket::new_with_socket(self.address, self.namespace, socket)?;
        if let Some(callbacks) = self.on {
            for (event, callback) in callbacks {
                socket.on(event, Box::new(callback)).unwrap();
            }
        }
        socket.connect()?;
        Ok(socket)
    }
}

impl Socket {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new<T: Into<String>>(
        address: T,
        namespace: Option<String>,
        engine_socket: EngineSocket,
    ) -> Result<Self> {
        Ok(Socket {
            socket: InnerSocket::new(address, namespace, engine_socket)?,
        })
    }

    pub(crate) fn new_with_socket<T: Into<String>>(
        address: T,
        namespace: Option<String>,
        engine_socket: EngineSocket,
    ) -> Result<Self> {
        Ok(Socket {
            socket: InnerSocket::new(address, namespace, engine_socket)?,
        })
    }

    /// Registers a new callback for a certain event. This returns an
    /// `Error::IllegalActionAfterOpen` error if the callback is registered
    /// after a call to the `connect` method.
    pub(crate) fn on<F>(&mut self, event: Event, callback: Box<F>) -> Result<()>
    where
        F: FnMut(Payload, Self) + 'static + Sync + Send,
    {
        self.socket.on(event, callback)
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server. Attention: it's not allowed to add a
    /// callback after a call to this method.
    pub(crate) fn connect(&mut self) -> Result<()> {
        self.socket.connect()
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
        self.socket.emit(event.into(), data.into())
    }

    /// Disconnects this client from the server by sending a `socket.io` closing
    /// packet.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload, Socket};
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
    /// socket.emit("foo", json_payload);
    ///
    /// // disconnect from the server
    /// socket.disconnect();
    ///
    /// ```
    pub fn disconnect(&mut self) -> Result<()> {
        self.socket.disconnect()
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
    /// # Example
    /// ```
    /// use rust_socketio::{SocketBuilder, Payload};
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
    ///
    /// let ack_callback = |message: Payload, _| {
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
        F: FnMut(Payload, Socket) + 'static + Send + Sync,
        E: Into<Event>,
        D: Into<Payload>,
    {
        self.socket
            .emit_with_ack(event.into(), data.into(), timeout, callback)
    }

    pub(crate) fn iter(&self) -> Iter {
        Iter {
            socket_iter: self.socket.iter(),
        }
    }
}

pub(crate) struct Iter<'a> {
    socket_iter: crate::socket::Iter<'a, rust_engineio::client::Socket>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Result<Packet>;
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        self.socket_iter.next()
    }
}

#[cfg(test)]
mod test {

    use std::thread::sleep;

    use super::*;
    use crate::{packet::PacketId, payload::Payload};
    use bytes::Bytes;
    use native_tls::TlsConnector;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn socket_io_integration() -> Result<()> {
        //TODO: check to make sure we are receiving packets rather than logging.
        let url = crate::test::socket_io_server()?;

        let mut socket = SocketBuilder::new(url)
            .on(
                "test".into(),
                Box::new(|msg, _| match msg {
                    Payload::String(str) => println!("Received string: {}", str),
                    Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
                }),
            )
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

        sleep(Duration::from_secs(10));

        assert!(socket.disconnect().is_ok());

        Ok(())
    }

    #[test]
    fn socket_io_builder_integration() -> Result<()> {
        let url = crate::test::socket_io_server()?;

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

        let ack_cb = |payload, _| {
            println!("Yehaa the ack got acked");
            println!("With data: {:#?}", payload);
        };

        assert!(socket
            .emit_with_ack("binary", json!("pls ack"), Duration::from_secs(1), ack_cb,)
            .is_ok());

        sleep(Duration::from_secs(5));

        SocketBuilder::new(url.clone())
            .transport_type(TransportType::Polling)
            .connect()?;
        SocketBuilder::new(url.clone())
            .transport_type(TransportType::Websocket)
            .connect()?;
        SocketBuilder::new(url.clone())
            .transport_type(TransportType::WebsocketUpgrade)
            .connect()?;
        SocketBuilder::new(url.clone())
            .transport_type(TransportType::Any)
            .connect()?;

        Ok(())
    }

    fn test_socketio_socket(socket: Socket) -> Result<()> {
        let mut socket = socket;

        assert!(socket
            .on(
                "test".into(),
                Box::new(|message, _| {
                    if let Payload::String(st) = message {
                        println!("{}", st)
                    }
                })
            )
            .is_ok());

        assert!(socket.on("Error".into(), Box::new(|_, _| {})).is_ok());

        assert!(socket.on("Connect".into(), Box::new(|_, _| {})).is_ok());

        assert!(socket.on("Close".into(), Box::new(|_, _| {})).is_ok());

        // Tests need to consume packets rather than forward to callbacks.
        socket.socket.connect_with_thread(false).unwrap();

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

        let ack_callback = |message: Payload, _| {
            println!("Yehaa! My ack got acked?");
            if let Payload::String(str) = message {
                println!("Received string ack");
                println!("Ack data: {}", str);
            }
        };

        assert!(socket
            .emit_with_ack(
                "test",
                Payload::String("123".to_owned()),
                Duration::from_secs(10),
                ack_callback
            )
            .is_ok());

        Ok(())
    }

    //TODO: Test with SocketBuilder once specifying connection without thread is possible.

    #[test]
    fn test_connection() -> Result<()> {
        let url = crate::test::socket_io_server()?;

        let engine_socket = EngineSocketBuilder::new(url.clone()).build_with_fallback()?;

        let socket = Socket::new(url, None, engine_socket)?;

        test_socketio_socket(socket)
    }

    #[test]
    fn test_connection_failable() -> Result<()> {
        let url = crate::test::socket_io_server()?;

        let engine_socket = EngineSocketBuilder::new(url.clone()).build()?;

        let socket = Socket::new(url, None, engine_socket)?;

        test_socketio_socket(socket)
    }

    //TODO: make all engineio code-paths reachable from engineio, (is_binary_attr is only used in socketio)
    #[test]
    fn test_connection_polling() -> Result<()> {
        let url = crate::test::socket_io_server()?;

        let engine_socket = EngineSocketBuilder::new(url.clone()).build_polling()?;

        let socket = Socket::new(url, None, engine_socket)?;

        test_socketio_socket(socket)
    }

    #[test]
    fn test_connection_websocket() -> Result<()> {
        let url = crate::test::socket_io_server()?;

        let engine_socket = EngineSocketBuilder::new(url.clone()).build_websocket()?;

        let socket = Socket::new(url, None, engine_socket)?;

        test_socketio_socket(socket)
    }

    // TODO: add secure socketio server
    /*
    #[test]
    fn test_connection_websocket_secure() -> Result<()> {
        let url = crate::socketio::test::socket_io_server()?;

        let engine_socket = EngineIoSocketBuilder::new(url.clone()).build()?;

        let socket = Socket::new(url, None, engine_socket)?;

        test_socketio_socket(socket)
    }
    */

    #[test]
    fn test_error_cases() -> Result<()> {
        let result = SocketBuilder::new("http://localhost:123").connect();
        assert!(result.is_err());
        Ok(())
    }
}
