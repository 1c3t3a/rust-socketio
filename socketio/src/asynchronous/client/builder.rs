use std::collections::HashMap;

use futures_util::{future::BoxFuture, StreamExt};
use log::trace;
use native_tls::TlsConnector;
use rust_engineio::{
    asynchronous::ClientBuilder as EngineIoClientBuilder,
    header::{HeaderMap, HeaderValue},
};
use url::Url;

use crate::{error::Result, Error, Event, Payload, TransportType};

use super::{callback::Callback, client::Client};
use crate::asynchronous::socket::Socket as InnerSocket;

/// A builder class for a `socket.io` socket. This handles setting up the client and
/// configuring the callback, the namespace and metadata of the socket. If no
/// namespace is specified, the default namespace `/` is taken. The `connect` method
/// acts the `build` method and returns a connected [`Client`].
pub struct ClientBuilder {
    address: String,
    on: HashMap<Event, Callback>,
    namespace: String,
    tls_config: Option<TlsConnector>,
    opening_headers: Option<HeaderMap>,
    transport_type: TransportType,
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
    ///                 Payload::String(str) => println!("Received: {}", str),
    ///                 Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
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
            namespace: "/".to_owned(),
            tls_config: None,
            opening_headers: None,
            transport_type: TransportType::Any,
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
    /// one of the common events like `message`, `error`, `connect`, `close` or a custom
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
    ///                        Payload::String(str) => println!("Received: {}", str),
    ///                       Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
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
    ///                        Payload::String(str) => println!("Received: {}", str),
    ///                       Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
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
        self.on.insert(event.into(), Callback::new(callback));
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
        let mut socket_clone = socket.clone();

        // Use thread to consume items in iterator in order to call callbacks
        tokio::runtime::Handle::current().spawn(async move {
            loop {
                // tries to restart a poll cycle whenever a 'normal' error occurs,
                // it just logs on network errors, in case the poll cycle returned
                // `Result::Ok`, the server receives a close frame so it's safe to
                // terminate
                if let Some(e @ Err(Error::IncompleteResponseFromEngineIo(_))) =
                    socket_clone.next().await
                {
                    trace!("Network error occured: {}", e.unwrap_err());
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

        let socket = Client::new(inner_socket, &self.namespace, self.on)?;
        socket.connect().await?;

        Ok(socket)
    }
}
