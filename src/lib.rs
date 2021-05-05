//! Rust-socket.io is a socket.io client written in the Rust Programming Language.
//! ## Example usage
//!
//! ``` rust
//! use rust_socketio::{SocketBuilder, Payload, Socket};
//! use serde_json::json;
//! use std::time::Duration;
//!
//! // define a callback which is called when a payload is received
//! // this callback gets the payload as well as an instance of the
//! // socket to communicate with the server
//! let callback = |payload: Payload, mut socket: Socket| {
//!        match payload {
//!            Payload::String(str) => println!("Received: {}", str),
//!            Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
//!        }
//!        socket.emit("test", json!({"got ack": true})).expect("Server unreachable")
//! };
//!
//! // get a socket that is connected to the admin namespace
//! let mut socket = SocketBuilder::new("http://localhost:4200")
//!      .set_namespace("/admin")
//!      .expect("illegal namespace")
//!      .on("test", callback)
//!      .on("error", |err, _| eprintln!("Error: {:#?}", err))
//!      .connect()
//!      .expect("Connection failed");
//!
//! // emit to the "foo" event
//! let json_payload = json!({"token": 123});
//!
//! socket.emit("foo", json_payload).expect("Server unreachable");
//!
//! // define a callback, that's executed when the ack got acked
//! let ack_callback = |message: Payload, _| {
//!     println!("Yehaa! My ack got acked?");
//!     println!("Ack data: {:#?}", message);
//! };
//!
//! let json_payload = json!({"myAckData": 123});
//!
//! // emit with an ack
//! let ack = socket
//!     .emit_with_ack("test", json_payload, Duration::from_secs(2), ack_callback)
//!     .expect("Server unreachable");
//! ```
//!
//! The main entry point for using this crate is the [`SocketBuilder`] which provides
//! a way to easily configure a socket in the needed way. When the `connect` method
//! is called on the builder, it returns a connected client which then could be used
//! to emit messages to certain events. One client can only be connected to one namespace.
//! If you need to listen to the messages in different namespaces you need to
//! allocate multiple sockets.
//!
//! ## Current features
//!
//! This implementation now supports all of the features of the socket.io protocol mentioned
//! [here](https://github.com/socketio/socket.io-protocol).
//! It generally tries to make use of websockets as often as possible. This means most times
//! only the opening request uses http and as soon as the server mentions that he is able to use
//! websockets, an upgrade  is performed. But if this upgrade is not successful or the server
//! does not mention an upgrade possibility, http-long polling is used (as specified in the protocol specs).
//!
//! Here's an overview of possible use-cases:
//!
//! - connecting to a server.
//! - register callbacks for the following event types:
//!     - open
//!     - close
//!     - error
//!     - message
//!     - custom events like "foo", "on_payment", etc.
//! - send JSON data to the server (via `serde_json` which provides safe
//! handling).
//! - send JSON data to the server and receive an `ack`.
//! - send and handle Binary data.
//!
#![allow(clippy::rc_buffer)]
#![warn(clippy::complexity)]
#![warn(clippy::style)]
#![warn(clippy::perf)]
#![warn(clippy::correctness)]
/// A small macro that spawns a scoped thread. Used for calling the callback
/// functions.
macro_rules! spawn_scoped {
    ($e:expr) => {
        crossbeam_utils::thread::scope(|s| {
            s.spawn(|_| $e);
        })
        .unwrap();
    };
}

/// Contains the types and the code concerning the `engine.io` protocol.
mod engineio;
/// Contains the types and the code concerning the `socket.io` protocol.
pub mod socketio;

/// Contains the error type which will be returned with every result in this
/// crate. Handles all kinds of errors.
pub mod error;

use error::Error;
use native_tls::TlsConnector;
pub use reqwest::header::{HeaderMap, HeaderValue, IntoHeaderName};
pub use socketio::{event::Event, payload::Payload};

use crate::error::Result;
use std::{time::Duration, vec};

use crate::socketio::transport::TransportClient;

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the server will connect to the default namespace `"/"`.
#[derive(Debug, Clone)]
pub struct Socket {
    /// The inner transport client to delegate the methods to.
    transport: TransportClient,
}

type SocketCallback = dyn FnMut(Payload, Socket) + 'static + Sync + Send;

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
    ///     .set_namespace("/admin")
    ///     .expect("illegal namespace")
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
        }
    }

    /// Sets the target namespace of the client. The namespace must start
    /// with a leading `/`. Valid examples are e.g. `/admin`, `/foo`.
    pub fn set_namespace<T: Into<String>>(mut self, namespace: T) -> Result<Self> {
        let nsp = namespace.into();
        if !nsp.starts_with('/') {
            return Err(Error::IllegalNamespace(nsp));
        }
        self.namespace = Some(nsp);
        Ok(self)
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
    /// let socket = SocketBuilder::new("http://localhost:4200")
    ///     .set_namespace("/admin")
    ///     .expect("illegal namespace")
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
    /// let socket = SocketBuilder::new("http://localhost:4200")
    ///     .set_namespace("/admin")
    ///     .expect("illegal namespace")
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .set_tls_config(tls_connector)
    ///     .connect();
    ///
    /// ```
    pub fn set_tls_config(mut self, tls_config: TlsConnector) -> Self {
        self.tls_config = Some(tls_config);
        self
    }

    /// Sets custom http headers for the opening request. The headers will definetly be send with the open
    /// request but not necessarly with every other request. When the transport type evaluates to `polling`
    /// the headers get send with every request, when websockets are used, not.
    pub fn set_opening_header<K: IntoHeaderName>(mut self, key: K, val: HeaderValue) -> Self {
        match self.opening_headers {
            Some(ref mut map) => {
                map.insert(key, val);
            }
            None => {
                let mut map = HeaderMap::new();
                map.insert(key, val);
                self.opening_headers = Some(map);
            }
        }
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
    /// let mut socket = SocketBuilder::new("http://localhost:4200")
    ///     .set_namespace("/admin")
    ///     .expect("illegal namespace")
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
        let mut socket = Socket::new(
            self.address,
            self.namespace,
            self.tls_config,
            self.opening_headers,
        );
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
    /// Creates a socket with a certain adress to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new<T: Into<String>>(
        address: T,
        namespace: Option<String>,
        tls_config: Option<TlsConnector>,
        opening_headers: Option<HeaderMap>,
    ) -> Self {
        Socket {
            transport: TransportClient::new(address, namespace, tls_config, opening_headers),
        }
    }

    /// Registers a new callback for a certain event. This returns an
    /// `Error::IllegalActionAfterOpen` error if the callback is registered
    /// after a call to the `connect` method.
    pub(crate) fn on<F>(&mut self, event: Event, callback: Box<F>) -> Result<()>
    where
        F: FnMut(Payload, Socket) + 'static + Sync + Send,
    {
        self.transport.on(event, callback)
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server. Attention: it's not allowed to add a
    /// callback after a call to this method.
    pub(crate) fn connect(&mut self) -> Result<()> {
        self.transport.connect()
    }

    /// Sends a message to the server using the underlying `engine.io` protocol.
    /// This message takes an event, which could either be one of the common
    /// events like "message" or "error" or a custom event like "foo". But be
    /// careful, the data string needs to be valid JSON. It's recommended to use
    /// a library like `serde_json` to serialize the data properly.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::{SocketBuilder, Payload};
    /// use serde_json::json;
    ///
    /// let mut socket = SocketBuilder::new("http://localhost:4200")
    ///     .on("test", |payload: Payload, mut socket| {
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
    pub fn emit<E, D>(&mut self, event: E, data: D) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        self.transport.emit(event.into(), data.into())
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
    /// let mut socket = SocketBuilder::new("http://localhost:4200")
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
        &mut self,
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
        self.transport
            .emit_with_ack(event.into(), data.into(), timeout, callback)
    }
}

#[cfg(test)]
mod test {

    use std::thread::sleep;

    use super::*;
    use bytes::Bytes;
    use native_tls::TlsConnector;
    use reqwest::header::{ACCEPT_ENCODING, HOST};
    use serde_json::json;
    const SERVER_URL: &str = "http://localhost:4200";

    #[test]
    fn it_works() {
        let mut socket = Socket::new(SERVER_URL, None, None, None);

        let result = socket.on(
            "test".into(),
            Box::new(|msg, _| match msg {
                Payload::String(str) => println!("Received string: {}", str),
                Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
            }),
        );
        assert!(result.is_ok());

        let result = socket.connect();
        assert!(result.is_ok());

        let payload = json!({"token": 123});
        let result = socket.emit("test", Payload::String(payload.to_string()));

        assert!(result.is_ok());

        let ack_callback = move |message: Payload, mut socket_: Socket| {
            let result = socket_.emit(
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

        sleep(Duration::from_secs(4));
    }

    #[test]
    fn test_builder() {
        // expect an illegal namespace
        assert!(SocketBuilder::new(SERVER_URL)
            .set_namespace("illegal")
            .is_err());

        // test socket build logic
        let socket_builder = SocketBuilder::new(SERVER_URL);

        let tls_connector = TlsConnector::builder()
            .use_sni(true)
            .build()
            .expect("Found illegal configuration");

        let socket = socket_builder
            .set_namespace("/")
            .expect("Error!")
            .set_tls_config(tls_connector)
            .set_opening_header(HOST, "localhost".parse().unwrap())
            .set_opening_header(ACCEPT_ENCODING, "application/json".parse().unwrap())
            .on("test", Box::new(|str, _| println!("Received: {:#?}", str)))
            .connect();

        assert!(socket.is_ok());

        let mut socket = socket.unwrap();
        assert!(socket.emit("message", json!("Hello World")).is_ok());

        assert!(socket.emit("binary", Bytes::from_static(&[46, 88])).is_ok());

        let ack_cb = |payload, _| {
            println!("Yehaa the ack got acked");
            println!("With data: {:#?}", payload);
        };

        assert!(socket
            .emit_with_ack("binary", json!("pls ack"), Duration::from_secs(1), ack_cb,)
            .is_ok());

        sleep(Duration::from_secs(2));
    }
}
