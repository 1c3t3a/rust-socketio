use super::super::{event::Event, payload::Payload};
use super::callback::Callback;
use crate::Client;
use native_tls::TlsConnector;
use rust_engineio::client::ClientBuilder as EngineIoClientBuilder;
use rust_engineio::header::{HeaderMap, HeaderValue};
use url::Url;

use crate::client::callback::{SocketAnyCallback, SocketCallback};
use crate::error::Result;
use crate::socket::Socket as InnerSocket;
use std::collections::HashMap;

/// Flavor of Engine.IO transport.
#[derive(Clone, Eq, PartialEq)]
pub enum TransportType {
    /// Handshakes with polling, upgrades if possible
    Any,
    /// Handshakes with websocket. Does not use polling.
    Websocket,
    /// Handshakes with polling, errors if upgrade fails
    WebsocketUpgrade,
    /// Handshakes with polling
    Polling,
}

/// A builder class for a `socket.io` socket. This handles setting up the client and
/// configuring the callback, the namespace and metadata of the socket. If no
/// namespace is specified, the default namespace `/` is taken. The `connect` method
/// acts the `build` method and returns a connected [`Client`].
pub struct ClientBuilder {
    address: String,
    on: HashMap<Event, Callback<SocketCallback>>,
    on_any: Option<Callback<SocketAnyCallback>>,
    namespace: String,
    tls_config: Option<TlsConnector>,
    opening_headers: Option<HeaderMap>,
    transport_type: TransportType,
    auth: Option<serde_json::Value>,
}

impl ClientBuilder {
    /// Create as client builder from a URL. URLs must be in the form
    /// `[ws or wss or http or https]://[domain]:[port]/[path]`. The
    /// path of the URL is optional and if no port is given, port 80
    /// will be used.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload, Client};
    /// use serde_json::json;
    ///
    ///
    /// let callback = |payload: Payload, socket: Client| {
    ///            match payload {
    ///                Payload::String(str) => println!("Received: {}", str),
    ///                Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///            }
    /// };
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200")
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
            on: HashMap::new(),
            on_any: None,
            namespace: "/".to_owned(),
            tls_config: None,
            opening_headers: None,
            transport_type: TransportType::Any,
            auth: None,
        }
    }

    /// Sets the target namespace of the client. The namespace should start
    /// with a leading `/`. Valid examples are e.g. `/admin`, `/foo`.
    pub fn namespace<T: Into<String>>(mut self, namespace: T) -> Self {
        let mut nsp = namespace.into();
        if !nsp.starts_with('/') {
            nsp = "/".to_owned() + &nsp;
        }
        self.namespace = nsp;
        self
    }

    /// Registers a new callback for a certain [`crate::event::Event`]. The event could either be
    /// one of the common events like `message`, `error`, `connect`, `close` or a custom
    /// event defined by a string, e.g. `onPayment` or `foo`.
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload};
    ///
    /// let socket = ClientBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on("test", |payload: Payload, _| {
    ///            match payload {
    ///                Payload::String(str) => println!("Received: {}", str),
    ///                Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///            }
    ///     })
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .connect();
    ///
    /// ```
    pub fn on<T: Into<Event>, F>(mut self, event: T, callback: F) -> Self
    where
        F: for<'a> FnMut(Payload, Client) + 'static + Sync + Send,
    {
        self.on
            .insert(event.into(), Callback::<SocketCallback>::new(callback));
        self
    }

    /// Registers a Callback for all [`crate::event::Event::Custom`] and [`crate::event::Event::Message`].
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload};
    ///
    /// let client = ClientBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on_any(|event, payload, _client| {
    ///         if let Payload::String(str) = payload {
    ///           println!("{} {}", String::from(event), str);
    ///         }
    ///     })
    ///     .connect();
    ///
    /// ```
    pub fn on_any<F>(mut self, callback: F) -> Self
    where
        F: for<'a> FnMut(Event, Payload, Client) + 'static + Sync + Send,
    {
        self.on_any = Some(Callback::<SocketAnyCallback>::new(callback));
        self
    }

    /// Uses a preconfigured TLS connector for secure communication. This configures
    /// both the `polling` as well as the `websocket` transport type.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload};
    /// use native_tls::TlsConnector;
    ///
    /// let tls_connector =  TlsConnector::builder()
    ///            .use_sni(true)
    ///            .build()
    ///            .expect("Found illegal configuration");
    ///
    /// let socket = ClientBuilder::new("http://localhost:4200/")
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
    /// use rust_socketio::{ClientBuilder, Payload};
    ///
    ///
    /// let socket = ClientBuilder::new("http://localhost:4200/")
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
                let mut map = HeaderMap::default();
                map.insert(key.into(), val.into());
                self.opening_headers = Some(map);
            }
        }
        self
    }

    /// Sets data sent in the opening request.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder};
    /// use serde_json::json;
    ///
    /// let socket = ClientBuilder::new("http://localhost:4204/")
    ///     .namespace("/admin")
    ///     .auth(json!({ "password": "1337" }))
    ///     .on("error", |err, _| eprintln!("Error: {:#?}", err))
    ///     .connect()
    ///     .expect("Connection error");
    ///
    /// ```
    pub fn auth(mut self, auth: serde_json::Value) -> Self {
        self.auth = Some(auth);

        self
    }

    /// Specifies which EngineIO [`TransportType`] to use.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, TransportType};
    ///
    /// let socket = ClientBuilder::new("http://localhost:4200/")
    ///     // Use websockets to handshake and connect.
    ///     .transport_type(TransportType::Websocket)
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// // use the socket
    pub fn transport_type(mut self, transport_type: TransportType) -> Self {
        self.transport_type = transport_type;

        self
    }

    /// Connects the socket to a certain endpoint. This returns a connected
    /// [`Client`] instance. This method returns an [`std::result::Result::Err`]
    /// value if something goes wrong during connection. Also starts a separate
    /// thread to start polling for packets. Used with callbacks.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload};
    /// use serde_json::json;
    ///
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .namespace("/admin")
    ///     .on("error", |err, _| eprintln!("Client error!: {:#?}", err))
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
    pub fn connect(self) -> Result<Client> {
        let socket = self.connect_manual()?;
        Ok(socket)
    }

    //TODO: 0.3.X stabilize
    pub(crate) fn connect_manual(self) -> Result<Client> {
        // Parse url here rather than in new to keep new returning Self.
        let mut url = Url::parse(&self.address)?;

        if url.path() == "/" {
            url.set_path("/socket.io/");
        }

        let mut builder = EngineIoClientBuilder::new(url);

        if let Some(tls_config) = self.tls_config {
            builder = builder.tls_config(tls_config);
        }
        if let Some(headers) = self.opening_headers {
            builder = builder.headers(headers);
        }

        let transport_type = self.transport_type.clone();
        let socket = Client::new(
            Box::new(move || build_socket(transport_type.clone(), builder.clone())),
            &self.namespace,
            self.on,
            self.on_any,
            self.auth,
        )?;

        socket.connect()?;

        Ok(socket)
    }
}

fn build_socket(
    transport_type: TransportType,
    builder: EngineIoClientBuilder,
) -> Result<InnerSocket> {
    let engine_client = match transport_type {
        TransportType::Any => builder.build_with_fallback()?,
        TransportType::Polling => builder.build_polling()?,
        TransportType::Websocket => builder.build_websocket()?,
        TransportType::WebsocketUpgrade => builder.build_websocket_with_upgrade()?,
    };

    InnerSocket::new(engine_client)
}
