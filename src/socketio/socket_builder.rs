use super::{Socket,payload::Payload,event::Event};
use crate::error::{Error,Result};
use native_tls::TlsConnector;
pub use reqwest::header::{HeaderMap, HeaderValue, IntoHeaderName};


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

    /// Sets custom http headers for the opening request. The headers will be passed to the underlying
    /// transport type (either websockets or polling) and then get passed with every request thats made.
    /// via the transport layer.
    /// # Example
    /// ```rust
    /// use rust_socketio::{SocketBuilder, Payload};
    /// use reqwest::header::{ACCEPT_ENCODING};
    ///
    ///
    /// let socket = SocketBuilder::new("http://localhost:4200")
    ///     .set_namespace("/admin")
    ///     .expect("illegal namespace")
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .set_opening_header(ACCEPT_ENCODING, "application/json".parse().unwrap())
    ///     .connect();
    ///
    /// ```
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
