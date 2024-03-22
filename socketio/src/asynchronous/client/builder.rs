use futures_util::{future::BoxFuture, StreamExt};
use log::trace;
use native_tls::TlsConnector;
use rust_engineio::{
    asynchronous::ClientBuilder as EngineIoClientBuilder,
    header::{HeaderMap, HeaderValue},
};
use std::collections::HashMap;
use url::Url;

use crate::{error::Result, Error, Event, Payload, TransportType};

use super::{
    callback::{Callback, DynAsyncAnyCallback, DynAsyncCallback},
    client::Client,
};
use crate::asynchronous::socket::Socket as InnerSocket;

/// A builder class for a `socket.io` socket. This handles setting up the client and
/// configuring the callback, the namespace and metadata of the socket. If no
/// namespace is specified, the default namespace `/` is taken. The `connect` method
/// acts the `build` method and returns a connected [`Client`].
pub struct ClientBuilder {
    address: String,
    on: HashMap<Event, Callback<DynAsyncCallback>>,
    on_any: Option<Callback<DynAsyncAnyCallback>>,
    namespace: String,
    tls_config: Option<TlsConnector>,
    opening_headers: Option<HeaderMap>,
    transport_type: TransportType,
    auth: Option<serde_json::Value>,
    pub(crate) reconnect: bool,
    pub(crate) reconnect_on_disconnect: bool,
    // None implies infinite attempts
    pub(crate) max_reconnect_attempts: Option<u8>,
    pub(crate) reconnect_delay_min: u64,
    pub(crate) reconnect_delay_max: u64,
}

impl ClientBuilder {
    /// Create as client builder from a URL. URLs must be in the form
    /// `[ws or wss or http or https]://[domain]:[port]/[path]`. The
    /// path of the URL is optional and if no port is given, port 80
    /// will be used.
    /// # Example
    /// ```rust
    /// use rust_socketio::{Payload, asynchronous::{ClientBuilder, Client}};
    /// use serde_json::json;
    /// use futures_util::future::FutureExt;
    ///
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let callback = |payload: Payload, socket: Client| {
    ///         async move {
    ///             match payload {
    ///                 Payload::Text(values) => println!("Received: {:#?}", values),
    ///                 Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///                 // This is deprecated, use Payload::Text instead
    ///                 Payload::String(str) => println!("Received: {}", str),
    ///             }
    ///         }.boxed()
    ///     };
    ///
    ///     let mut socket = ClientBuilder::new("http://localhost:4200")
    ///         .namespace("/admin")
    ///         .on("test", callback)
    ///         .connect()
    ///         .await
    ///         .expect("error while connecting");
    ///
    ///     // use the socket
    ///     let json_payload = json!({"token": 123});
    ///
    ///     let result = socket.emit("foo", json_payload).await;
    ///
    ///     assert!(result.is_ok());
    /// }
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
            reconnect: true,
            reconnect_on_disconnect: false,
            // None implies infinite attempts
            max_reconnect_attempts: None,
            reconnect_delay_min: 1000,
            reconnect_delay_max: 5000,
        }
    }

    /// Sets the target namespace of the client. The namespace should start
    /// with a leading `/`. Valid examples are e.g. `/admin`, `/foo`.
    /// If the String provided doesn't start with a leading `/`, it is
    /// added manually.
    pub fn namespace<T: Into<String>>(mut self, namespace: T) -> Self {
        let mut nsp = namespace.into();
        if !nsp.starts_with('/') {
            nsp = "/".to_owned() + &nsp;
            trace!("Added `/` to the given namespace: {}", nsp);
        }
        self.namespace = nsp;
        self
    }

    /// Registers a new callback for a certain [`crate::event::Event`]. The event could either be
    /// one of the common events like `message`, `error`, `open`, `close` or a custom
    /// event defined by a string, e.g. `onPayment` or `foo`.
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder, Payload};
    /// use futures_util::FutureExt;
    ///
    ///  #[tokio::main]
    /// async fn main() {
    ///     let socket = ClientBuilder::new("http://localhost:4200/")
    ///         .namespace("/admin")
    ///         .on("test", |payload: Payload, _| {
    ///             async move {
    ///                 match payload {
    ///                     Payload::Text(values) => println!("Received: {:#?}", values),
    ///                     Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///                     // This is deprecated, use Payload::Text instead
    ///                     Payload::String(str) => println!("Received: {}", str),
    ///                 }
    ///             }
    ///             .boxed()
    ///         })
    ///         .on("error", |err, _| async move { eprintln!("Error: {:#?}", err) }.boxed())
    ///         .connect()
    ///         .await;
    /// }
    /// ```
    ///
    /// # Issues with type inference for the callback method
    ///
    /// Currently stable Rust does not contain types like `AsyncFnMut`.
    /// That is why this library uses the type `FnMut(..) -> BoxFuture<_>`,
    /// which basically represents a closure or function that returns a
    /// boxed future that can be executed in an async executor.
    /// The complicated constraints for the callback function
    /// bring the Rust compiler to it's limits, resulting in confusing error
    /// messages when passing in a variable that holds a closure (to the `on` method).
    /// In order to make sure type inference goes well, the [`futures_util::FutureExt::boxed`]
    /// method can be used on an async block (the future) to make sure the return type
    /// is conform with the generic requirements. An example can be found here:
    ///
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder, Payload};
    /// use futures_util::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let callback = |payload: Payload, _| {
    ///             async move {
    ///                 match payload {
    ///                     Payload::Text(values) => println!("Received: {:#?}", values),
    ///                     Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
    ///                     // This is deprecated use Payload::Text instead
    ///                     Payload::String(str) => println!("Received: {}", str),
    ///                 }
    ///             }
    ///             .boxed() // <-- this makes sure we end up with a `BoxFuture<_>`
    ///         };
    ///
    ///     let socket = ClientBuilder::new("http://localhost:4200/")
    ///         .namespace("/admin")
    ///         .on("test", callback)
    ///         .connect()
    ///         .await;
    /// }
    /// ```
    ///
    #[cfg(feature = "async-callbacks")]
    pub fn on<T: Into<Event>, F>(mut self, event: T, callback: F) -> Self
    where
        F: for<'a> std::ops::FnMut(Payload, Client) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
    {
        self.on
            .insert(event.into(), Callback::<DynAsyncCallback>::new(callback));
        self
    }

    /// Registers a Callback for all [`crate::event::Event::Custom`] and [`crate::event::Event::Message`].
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder, Payload};
    /// use futures_util::future::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let client = ClientBuilder::new("http://localhost:4200/")
    ///         .namespace("/admin")
    ///         .on_any(|event, payload, _client| {
    ///             async {
    ///                 if let Payload::String(str) = payload {
    ///                     println!("{}: {}", String::from(event), str);
    ///                 }
    ///             }.boxed()
    ///         })
    ///         .connect()
    ///         .await;
    /// }
    /// ```
    pub fn on_any<F>(mut self, callback: F) -> Self
    where
        F: for<'a> FnMut(Event, Payload, Client) -> BoxFuture<'static, ()> + 'static + Send + Sync,
    {
        self.on_any = Some(Callback::<DynAsyncAnyCallback>::new(callback));
        self
    }

    /// Uses a preconfigured TLS connector for secure communication. This configures
    /// both the `polling` as well as the `websocket` transport type.
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder, Payload};
    /// use native_tls::TlsConnector;
    /// use futures_util::future::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let tls_connector =  TlsConnector::builder()
    ///                .use_sni(true)
    ///                .build()
    ///             .expect("Found illegal configuration");
    ///
    ///     let socket = ClientBuilder::new("http://localhost:4200/")
    ///         .namespace("/admin")
    ///         .on("error", |err, _| async move { eprintln!("Error: {:#?}", err) }.boxed())
    ///         .tls_config(tls_connector)
    ///         .connect()
    ///         .await;
    /// }
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
    /// use rust_socketio::{asynchronous::ClientBuilder, Payload};
    /// use futures_util::future::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let socket = ClientBuilder::new("http://localhost:4200/")
    ///         .namespace("/admin")
    ///         .on("error", |err, _| async move { eprintln!("Error: {:#?}", err) }.boxed())
    ///         .opening_header("accept-encoding", "application/json")
    ///         .connect()
    ///         .await;
    /// }
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

    /// Sets authentification data sent in the opening request.
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder};
    /// use serde_json::json;
    /// use futures_util::future::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let socket = ClientBuilder::new("http://localhost:4204/")
    ///         .namespace("/admin")
    ///         .auth(json!({ "password": "1337" }))
    ///         .on("error", |err, _| async move { eprintln!("Error: {:#?}", err) }.boxed())
    ///         .connect()
    ///         .await;
    /// }
    /// ```
    pub fn auth<T: Into<serde_json::Value>>(mut self, auth: T) -> Self {
        self.auth = Some(auth.into());

        self
    }

    /// Specifies which EngineIO [`TransportType`] to use.
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder, TransportType};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let socket = ClientBuilder::new("http://localhost:4200/")
    ///         // Use websockets to handshake and connect.
    ///         .transport_type(TransportType::Websocket)
    ///         .connect()
    ///         .await
    ///         .expect("connection failed");
    /// }
    /// ```
    pub fn transport_type(mut self, transport_type: TransportType) -> Self {
        self.transport_type = transport_type;

        self
    }

    /// If set to `false` do not try to reconnect on network errors. Defaults to
    /// `true`
    pub fn reconnect(mut self, reconnect: bool) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// If set to `true` try to reconnect when the server disconnects the
    /// client. Defaults to `false`
    pub fn reconnect_on_disconnect(mut self, reconnect_on_disconnect: bool) -> Self {
        self.reconnect_on_disconnect = reconnect_on_disconnect;
        self
    }

    /// Sets the minimum and maximum delay between reconnection attempts
    pub fn reconnect_delay(mut self, min: u64, max: u64) -> Self {
        self.reconnect_delay_min = min;
        self.reconnect_delay_max = max;
        self
    }

    /// Sets the maximum number of times to attempt reconnections. Defaults to
    /// an infinite number of attempts
    pub fn max_reconnect_attempts(mut self, reconnect_attempts: u8) -> Self {
        self.max_reconnect_attempts = Some(reconnect_attempts);
        self
    }

    /// Connects the socket to a certain endpoint. This returns a connected
    /// [`Client`] instance. This method returns an [`std::result::Result::Err`]
    /// value if something goes wrong during connection. Also starts a separate
    /// thread to start polling for packets. Used with callbacks.
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::ClientBuilder, Payload};
    /// use serde_json::json;
    /// use futures_util::future::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///         .namespace("/admin")
    ///         .on("error", |err, _| async move { eprintln!("Error: {:#?}", err) }.boxed())
    ///         .connect()
    ///         .await
    ///         .expect("connection failed");
    ///
    ///     // use the socket
    ///     let json_payload = json!({"token": 123});
    ///
    ///     let result = socket.emit("foo", json_payload).await;
    ///
    ///     assert!(result.is_ok());
    /// }
    /// ```
    pub async fn connect(self) -> Result<Client> {
        let socket = self.connect_manual().await?;
        let socket_clone = socket.clone();

        // Use thread to consume items in iterator in order to call callbacks
        tokio::runtime::Handle::current().spawn(async move {
            let mut stream = socket_clone.as_stream();
            // Consume the stream until it returns None and the stream is closed.
            while let Some(item) = stream.next().await {
                if let e @ Err(Error::IncompleteResponseFromEngineIo(_)) = item {
                    trace!("Network error occurred: {}", e.unwrap_err());
                }
            }
        });

        Ok(socket)
    }

    //TODO: 0.3.X stabilize
    pub(crate) async fn connect_manual(self) -> Result<Client> {
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

        let engine_client = match self.transport_type {
            TransportType::Any => builder.build_with_fallback().await?,
            TransportType::Polling => builder.build_polling().await?,
            TransportType::Websocket => builder.build_websocket().await?,
            TransportType::WebsocketUpgrade => builder.build_websocket_with_upgrade().await?,
        };

        let inner_socket = InnerSocket::new(engine_client)?;

        let socket = Client::new(
            inner_socket,
            &self.namespace,
            self.on,
            self.on_any,
            self.auth,
        )?;
        socket.connect().await?;

        Ok(socket)
    }
}
