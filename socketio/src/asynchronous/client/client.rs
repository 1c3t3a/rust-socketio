use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    task::Poll,
};

use futures_util::{future::BoxFuture, ready, FutureExt, Stream, StreamExt};
use log::trace;
use serde_json::{from_str, Value};
use tokio::{
    sync::RwLock,
    time::{Duration, Instant},
};

use crate::{
    asynchronous::{
        ack::{Ack, AckId},
        callback::Callback,
        socket::Socket as InnerSocket,
    },
    error::{Error, Result},
    packet::{AckIdGenerator, Packet, PacketId},
    Event, Payload,
};

#[derive(Clone)]
pub struct Client {
    inner: CommonClient<Self>,
}

impl Deref for Client {
    type Target = CommonClient<Self>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Client {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Client {
    pub(crate) fn new<T: Into<String>>(
        socket: InnerSocket,
        namespace: T,
        on: Arc<RwLock<HashMap<Event, Callback<Self>>>>,
    ) -> Self {
        Self {
            inner: CommonClient::new(socket, namespace, on, Arc::new(|inner| Client { inner })),
        }
    }
}

// TODO: move SharedClient to common file.

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the client will connect to the default namespace `"/"`. Both client side
/// and server side, use this Client.
#[derive(Clone)]
pub struct CommonClient<C> {
    // namespace, for multiplexing messages
    pub(crate) nsp: String,
    /// The inner socket client to delegate the methods to.
    socket: InnerSocket,
    on: Arc<RwLock<HashMap<Event, Callback<C>>>>,
    outstanding_acks: Arc<RwLock<Vec<Ack<C>>>>,
    is_connected: Arc<AtomicBool>,
    callback_client_fn: Arc<dyn Fn(Self) -> C + Send + Sync>,
    ack_id_gen: Arc<AckIdGenerator>,
}

impl<C: Clone> CommonClient<C> {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new<T: Into<String>>(
        socket: InnerSocket,
        namespace: T,
        on: Arc<RwLock<HashMap<Event, Callback<C>>>>,
        callback_client_fn: Arc<dyn Fn(Self) -> C + Send + Sync>,
    ) -> Self {
        CommonClient {
            socket,
            nsp: namespace.into(),
            on,
            outstanding_acks: Arc::new(RwLock::new(Vec::new())),
            is_connected: Arc::new(AtomicBool::new(true)),
            callback_client_fn,
            ack_id_gen: Default::default(),
        }
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server.
    pub(crate) async fn connect(&self) -> Result<()> {
        // Connect the underlying socket
        self.socket.connect().await?;

        // construct the opening packet
        let open_packet = Packet::new(PacketId::Connect, self.nsp.clone(), None, None, 0, None);

        self.socket.send(open_packet).await?;

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
    /// use rust_socketio::{asynchronous::{ClientBuilder, Client, AckId}, Payload};
    /// use serde_json::json;
    /// use futures_util::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///         .on("test", |payload: Payload, socket: Client, need_ack: Option<AckId>| {
    ///             async move {
    ///                 println!("Received: {:#?}", payload);
    ///                 socket.emit("test", json!({"hello": true})).await.expect("Server unreachable");
    ///             }.boxed()
    ///         })
    ///         .connect()
    ///         .await
    ///         .expect("connection failed");
    ///
    ///     let json_payload = json!({"token": 123});
    ///
    ///     let result = socket.emit("foo", json_payload).await;
    ///
    ///     assert!(result.is_ok());
    /// }
    /// ```
    #[inline]
    pub async fn emit<E, D>(&self, event: E, data: D) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        if !self.is_connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionBeforeOpen());
        }
        self.socket.emit(&self.nsp, event.into(), data.into()).await
    }

    #[inline]
    pub async fn ack<D>(&self, id: usize, data: D) -> Result<()>
    where
        D: Into<Payload>,
    {
        if !self.is_connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionBeforeOpen());
        }
        self.socket.ack(&self.nsp, id, data.into()).await
    }

    #[inline]
    pub(crate) async fn handshake(&self, data: String) -> Result<()> {
        if !self.is_connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionBeforeOpen());
        }
        self.socket.handshake(&self.nsp, data).await
    }

    /// Disconnects this client from the server by sending a `socket.io` closing
    /// packet.
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::{ClientBuilder, Client, AckId}, Payload};
    /// use serde_json::json;
    /// use futures_util::{FutureExt, future::BoxFuture};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // apparently the syntax for functions is a bit verbose as rust currently doesn't
    ///     // support an `AsyncFnMut` type that conform with async functions
    ///     fn handle_test(payload: Payload, socket: Client, need_ack: Option<AckId>) -> BoxFuture<'static, ()> {
    ///         async move {
    ///             println!("Received: {:#?}", payload);
    ///             socket.emit("test", json!({"hello": true})).await.expect("Server unreachable");
    ///         }.boxed()
    ///     }
    ///
    ///     let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///         .on("test", handle_test)
    ///         .connect()
    ///         .await
    ///         .expect("connection failed");
    ///
    ///     let json_payload = json!({"token": 123});
    ///
    ///     socket.emit("foo", json_payload).await;
    ///
    ///     // disconnect from the server
    ///     socket.disconnect().await;
    /// }
    /// ```
    pub async fn disconnect(&self) -> Result<()> {
        if self.is_connected.load(Ordering::Acquire) {
            self.is_connected.store(false, Ordering::Release);
        }
        let disconnect_packet =
            Packet::new(PacketId::Disconnect, self.nsp.clone(), None, None, 0, None);

        self.socket.send(disconnect_packet).await?;
        self.socket.disconnect().await?;

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
    /// Please note that the requirements on the provided callbacks are similar to the ones
    /// for [`crate::asynchronous::ClientBuilder::on`].
    /// # Example
    /// ```
    /// use rust_socketio::{asynchronous::{ClientBuilder, Client}, Payload};
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use std::thread::sleep;
    /// use futures_util::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///         .on("foo", |payload: Payload, _, _| async move { println!("Received: {:#?}", payload) }.boxed())
    ///         .connect()
    ///         .await
    ///         .expect("connection failed");
    ///
    ///     let ack_callback = |message: Payload, socket: Client, _| {
    ///         async move {
    ///             match message {
    ///                 Payload::String(str) => println!("{}", str),
    ///                 Payload::Binary(bytes) => println!("Received bytes: {:#?}", bytes),
    ///             }
    ///         }.boxed()
    ///     };    
    ///
    ///
    ///     let payload = json!({"token": 123});
    ///     socket.emit_with_ack("foo", payload, Duration::from_secs(2), ack_callback).await.unwrap();
    ///
    ///     sleep(Duration::from_secs(2));
    /// }
    /// ```
    #[inline]
    pub async fn emit_with_ack<F, E, D>(
        &self,
        event: E,
        data: D,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: for<'a> std::ops::FnMut(Payload, C, Option<AckId>) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
        E: Into<Event>,
        D: Into<Payload>,
    {
        if !self.is_connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionBeforeOpen());
        }
        let id = self.ack_id_gen.generate();
        let socket_packet = self.socket.build_packet_for_payload(
            data.into(),
            Some(event.into()),
            &self.nsp,
            Some(id),
            false,
        )?;

        let ack = Ack {
            id,
            time_started: Instant::now(),
            timeout,
            callback: Callback::new(callback),
        };

        // add the ack to the tuple of outstanding acks
        self.outstanding_acks.write().await.push(ack);

        self.socket.send(socket_packet).await
    }

    async fn callback<P: Into<Payload>>(
        &self,
        event: &Event,
        payload: P,
        need_ack: Option<AckId>,
    ) -> Result<()> {
        let mut on = self.on.write().await;
        let lock = on.deref_mut();
        if let Some(callback) = lock.get_mut(event) {
            let c = (self.callback_client_fn)((self).clone());
            callback(payload.into(), c, need_ack).await;
        }
        drop(on);
        Ok(())
    }

    /// Handles the incoming acks and classifies what callbacks to call and how.
    #[inline]
    async fn handle_ack(&self, socket_packet: &Packet) -> Result<()> {
        let mut to_be_removed = Vec::new();
        if let Some(id) = socket_packet.id {
            for (index, ack) in self.outstanding_acks.write().await.iter_mut().enumerate() {
                if ack.id == id {
                    to_be_removed.push(index);

                    if ack.time_started.elapsed() < ack.timeout {
                        if let Some(ref payload) = socket_packet.data {
                            ack.callback.deref_mut()(
                                Payload::String(payload.to_owned()),
                                (self.callback_client_fn)(self.clone()),
                                None,
                            )
                            .await;
                        }
                        if let Some(ref attachments) = socket_packet.attachments {
                            if let Some(payload) = attachments.get(0) {
                                ack.callback.deref_mut()(
                                    Payload::Binary(payload.to_owned()),
                                    (self.callback_client_fn)(self.clone()),
                                    None,
                                )
                                .await;
                            }
                        }
                    } else {
                        trace!("Received an Ack that is now timed out (elapsed time was longer than specified duration)");
                    }
                }
            }
            for index in to_be_removed {
                self.outstanding_acks.write().await.remove(index);
            }
        }
        Ok(())
    }

    /// Handles a binary event.
    #[inline]
    async fn handle_binary_event(&self, packet: &Packet) -> Result<()> {
        let event = if let Some(string_data) = &packet.data {
            string_data.replace('\"', "").into()
        } else {
            Event::Message
        };

        if let Some(attachments) = &packet.attachments {
            if let Some(binary_payload) = attachments.get(0) {
                self.callback(
                    &event,
                    Payload::Binary(binary_payload.to_owned()),
                    packet.id,
                )
                .await?;
            }
        }
        Ok(())
    }

    /// A method that parses a packet and eventually calls the corresponding
    // callback with the supplied data.
    async fn handle_event(&self, packet: &Packet) -> Result<()> {
        if packet.data.is_none() {
            return Ok(());
        }
        let data = packet.data.as_ref().unwrap();

        // a socketio message always comes in one of the following two flavors (both JSON):
        // 1: `["event", "msg"]`
        // 2: `["msg"]`
        // in case 2, the message is ment for the default message event, in case 1 the event
        // is specified
        if let Ok(Value::Array(contents)) = from_str::<Value>(data) {
            let (event, data) = if contents.len() > 1 {
                // case 1
                (
                    contents
                        .get(0)
                        .map(|value| match value {
                            Value::String(ev) => ev,
                            _ => "message",
                        })
                        .unwrap_or("message")
                        .into(),
                    contents.get(1).ok_or(Error::IncompletePacket())?,
                )
            } else {
                // case 2
                (
                    Event::Message,
                    contents.get(0).ok_or(Error::IncompletePacket())?,
                )
            };

            // call the correct callback
            self.callback(&event, data.to_string(), packet.id).await?;
        }

        Ok(())
    }

    /// Handles the incoming messages and classifies what callbacks to call and how.
    /// This method is later registered as the callback for the `on_data` event of the
    /// engineio client.
    #[inline]
    async fn handle_socketio_packet(&self, packet: &Packet) -> Result<()> {
        if packet.nsp == self.nsp {
            match packet.packet_type {
                PacketId::Ack | PacketId::BinaryAck => {
                    if let Err(err) = self.handle_ack(packet).await {
                        self.callback(&Event::Error, err.to_string(), None).await?;
                        return Err(err);
                    }
                }
                PacketId::BinaryEvent => {
                    if let Err(err) = self.handle_binary_event(packet).await {
                        self.callback(&Event::Error, err.to_string(), None).await?;
                    }
                }
                PacketId::Connect => {
                    self.is_connected.store(true, Ordering::Release);
                    self.callback(&Event::Connect, "", None).await?;
                }
                PacketId::Disconnect => {
                    self.is_connected.store(false, Ordering::Release);
                    self.callback(&Event::Close, "", None).await?;
                }
                PacketId::ConnectError => {
                    self.is_connected.store(false, Ordering::Release);
                    self.callback(
                        &Event::Error,
                        String::from("Received an ConnectError frame: ")
                            + packet
                                .data
                                .as_ref()
                                .unwrap_or(&String::from("\"No error message provided\"")),
                        None,
                    )
                    .await?;
                }
                PacketId::Event => {
                    if let Err(err) = self.handle_event(packet).await {
                        self.callback(&Event::Error, err.to_string(), None).await?;
                    }
                }
            }
        }
        Ok(())
    }
}

impl<C: Clone> Stream for CommonClient<C> {
    type Item = Result<Packet>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        loop {
            // poll for the next payload
            let next = ready!(self.socket.poll_next_unpin(cx));
            match next {
                None => {
                    // end the stream if the underlying one is closed
                    return Poll::Ready(None);
                }
                Some(Err(err)) => {
                    // call the error callback
                    ready!(
                        Box::pin(self.callback(&Event::Error, err.to_string(), None))
                            .poll_unpin(cx)
                    )?;
                    return Poll::Ready(Some(Err(err)));
                }
                Some(Ok(packet)) => {
                    // if this packet is not meant for the current namespace, skip it an poll for the next one
                    if packet.nsp == self.nsp {
                        ready!(Box::pin(self.handle_socketio_packet(&packet)).poll_unpin(cx))?;
                        return Poll::Ready(Some(Ok(packet)));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {

    use std::time::Duration;

    use bytes::Bytes;
    use futures_util::{FutureExt, StreamExt};
    use native_tls::TlsConnector;
    use serde_json::json;
    use tokio::time::sleep;

    use crate::{
        asynchronous::client::{builder::ClientBuilder, client::Client},
        error::Result,
        packet::{Packet, PacketId},
        Payload, TransportType,
    };

    #[tokio::test]
    async fn socket_io_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        let socket = ClientBuilder::new(url)
            .on("test", |msg, _, _| {
                async {
                    match msg {
                        Payload::String(str) => println!("Received string: {}", str),
                        Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
                    }
                }
                .boxed()
            })
            .connect()
            .await?;

        let payload = json!({"token": 123_i32});
        let result = socket
            .emit("test", Payload::String(payload.to_string()))
            .await;

        assert!(result.is_ok());

        let ack = socket
            .emit_with_ack(
                "test",
                Payload::String(payload.to_string()),
                Duration::from_secs(1),
                |message: Payload, socket: Client, _| {
                    async move {
                        let result = socket
                            .emit(
                                "test",
                                Payload::String(json!({"got ack": true}).to_string()),
                            )
                            .await;
                        assert!(result.is_ok());

                        println!("Yehaa! My ack got acked?");
                        if let Payload::String(str) = message {
                            println!("Received string Ack");
                            println!("Ack data: {}", str);
                        }
                    }
                    .boxed()
                },
            )
            .await;
        assert!(ack.is_ok());

        sleep(Duration::from_secs(2)).await;

        assert!(socket.disconnect().await.is_ok());

        Ok(())
    }

    #[tokio::test]
    async fn socket_io_builder_integration() -> Result<()> {
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
            .on("test", |str, _, _| {
                async move { println!("Received: {:#?}", str) }.boxed()
            })
            .on("message", |payload, _, _| {
                async move { println!("{:#?}", payload) }.boxed()
            })
            .connect()
            .await?;

        assert!(socket.emit("message", json!("Hello World")).await.is_ok());

        assert!(socket
            .emit("binary", Bytes::from_static(&[46, 88]))
            .await
            .is_ok());

        assert!(socket
            .emit_with_ack(
                "binary",
                json!("pls ack"),
                Duration::from_secs(1),
                |payload, _, _| async move {
                    println!("Yehaa the ack got acked");
                    println!("With data: {:#?}", payload);
                }
                .boxed()
            )
            .await
            .is_ok());

        sleep(Duration::from_secs(2)).await;

        Ok(())
    }

    #[tokio::test]
    async fn socket_io_builder_integration_iterator() -> Result<()> {
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
            .on("test", |str, _, _| {
                async move { println!("Received: {:#?}", str) }.boxed()
            })
            .on("message", |payload, _, _| {
                async move { println!("{:#?}", payload) }.boxed()
            })
            .connect_manual()
            .await?;

        assert!(socket.emit("message", json!("Hello World")).await.is_ok());

        assert!(socket
            .emit("binary", Bytes::from_static(&[46, 88]))
            .await
            .is_ok());

        assert!(socket
            .emit_with_ack(
                "binary",
                json!("pls ack"),
                Duration::from_secs(1),
                |payload, _, _| async move {
                    println!("Yehaa the ack got acked");
                    println!("With data: {:#?}", payload);
                }
                .boxed()
            )
            .await
            .is_ok());

        test_socketio_socket(socket, "/admin".to_owned()).await
    }

    #[tokio::test]
    async fn socketio_polling_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url.clone())
            .transport_type(TransportType::Polling)
            .connect_manual()
            .await?;
        test_socketio_socket(socket, "/".to_owned()).await
    }

    #[tokio::test]
    async fn socket_io_websocket_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url.clone())
            .transport_type(TransportType::Websocket)
            .connect_manual()
            .await?;
        test_socketio_socket(socket, "/".to_owned()).await
    }

    #[tokio::test]
    async fn socket_io_websocket_upgrade_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::WebsocketUpgrade)
            .connect_manual()
            .await?;
        test_socketio_socket(socket, "/".to_owned()).await
    }

    #[tokio::test]
    async fn socket_io_any_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let socket = ClientBuilder::new(url)
            .transport_type(TransportType::Any)
            .connect_manual()
            .await?;
        test_socketio_socket(socket, "/".to_owned()).await
    }

    async fn test_socketio_socket(mut socket: Client, nsp: String) -> Result<()> {
        // open packet
        let _: Option<Packet> = Some(socket.next().await.unwrap()?);

        let packet: Option<Packet> = Some(socket.next().await.unwrap()?);

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

        let packet: Option<Packet> = Some(socket.next().await.unwrap()?);

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
        let packet: Option<Packet> = Some(socket.next().await.unwrap()?);

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

        let packet: Option<Packet> = Some(socket.next().await.unwrap()?);

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

        let cb = |message: Payload, _, _| {
            async {
                println!("Yehaa! My ack got acked?");
                if let Payload::String(str) = message {
                    println!("Received string ack");
                    println!("Ack data: {}", str);
                }
            }
            .boxed()
        };

        assert!(socket
            .emit_with_ack(
                "test",
                Payload::String("123".to_owned()),
                Duration::from_secs(10),
                cb
            )
            .await
            .is_ok());

        Ok(())
    }
}
