use std::{
    ops::DerefMut,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use backoff::{backoff::Backoff, ExponentialBackoffBuilder};
use futures_util::{future::BoxFuture, stream, Stream, StreamExt};
use log::trace;
use rand::{thread_rng, Rng};
use serde_json::Value;
use tokio::{
    sync::RwLock,
    time::{sleep, Duration, Instant},
};

use super::{
    ack::Ack,
    builder::ClientBuilder,
    callback::{Callback, DynAsyncCallback},
};
use crate::{
    asynchronous::socket::Socket as InnerSocket,
    error::{Error, Result},
    packet::{Packet, PacketId},
    Event, Payload,
};

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the client will connect to the default namespace `"/"`.
#[derive(Clone)]
pub struct Client {
    /// The inner socket client to delegate the methods to.
    socket: Arc<RwLock<InnerSocket>>,
    outstanding_acks: Arc<RwLock<Vec<Ack>>>,
    // namespace, for multiplexing messages
    nsp: String,
    // Data send in the opening packet (commonly used as for auth)
    auth: Option<serde_json::Value>,
    builder: Arc<RwLock<ClientBuilder>>,
    manually_disconnected: Arc<AtomicBool>,
    server_disconnected: Arc<AtomicBool>,
}

impl Client {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) fn new(socket: InnerSocket, builder: ClientBuilder) -> Result<Self> {
        Ok(Client {
            socket: Arc::new(RwLock::new(socket)),
            nsp: builder.namespace.to_owned(),
            outstanding_acks: Arc::new(RwLock::new(Vec::new())),
            auth: builder.auth.clone(),
            builder: Arc::new(RwLock::new(builder)),
            manually_disconnected: Arc::new(AtomicBool::new(false)),
            server_disconnected: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server.
    pub(crate) async fn connect(&self) -> Result<()> {
        // Connect the underlying socket
        self.socket.read().await.connect().await?;

        // construct the opening packet
        let auth = self.auth.as_ref().map(|data| data.to_string());
        let open_packet = Packet::new(PacketId::Connect, self.nsp.clone(), auth, None, 0, None);

        self.socket.read().await.send(open_packet).await?;

        Ok(())
    }

    pub(crate) async fn reconnect(&mut self) -> Result<()> {
        let builder = self.builder.write().await;
        let socket = builder.inner_create().await?;

        // New inner socket that can be connected
        let mut client_socket = self.socket.write().await;
        *client_socket = socket;
        drop(client_socket);

        self.connect().await?;

        Ok(())
    }

    /// Drives the stream using a thread so messages are processed
    pub(crate) async fn poll_stream(&mut self) -> Result<()> {
        let builder = self.builder.read().await;
        let reconnect_delay_min = builder.reconnect_delay_min;
        let reconnect_delay_max = builder.reconnect_delay_max;
        let max_reconnect_attempts = builder.max_reconnect_attempts;
        drop(builder);

        let mut client_clone = self.clone();

        tokio::runtime::Handle::current().spawn(async move {
            loop {
                let mut stream = client_clone.as_stream();
                // Consume the stream until it returns None and the stream is closed.
                while let Some(item) = stream.next().await {
                    if let Err(e) = item {
                        trace!("Network error occurred: {}", e);
                    }
                }

                // Drop the stream so we can once again use `socket_clone` as mutable
                drop(stream);

                if client_clone.should_reconnect().await {
                    let mut reconnect_attempts = 0;
                    let mut backoff = ExponentialBackoffBuilder::new()
                        .with_initial_interval(Duration::from_millis(reconnect_delay_min))
                        .with_max_interval(Duration::from_millis(reconnect_delay_max))
                        .build();

                    loop {
                        if let Some(max_reconnect_attempts) = max_reconnect_attempts {
                            reconnect_attempts += 1;
                            if reconnect_attempts > max_reconnect_attempts {
                                trace!("Max reconnect attempts reached without success");
                                break;
                            }
                        }
                        match client_clone.reconnect().await {
                            Ok(_) => {
                                trace!("Reconnected after {reconnect_attempts} attempts");
                                break;
                            }
                            Err(e) => {
                                trace!("Failed to reconnect: {e:?}");
                                if let Some(delay) = backoff.next_backoff() {
                                    let delay_ms = delay.as_millis();
                                    trace!("Waiting for {delay_ms}ms before reconnecting");
                                    sleep(delay).await;
                                }
                            }
                        }
                    }
                } else {
                    break;
                }
            }
        });

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
    /// use rust_socketio::{asynchronous::{ClientBuilder, Client}, Payload};
    /// use serde_json::json;
    /// use futures_util::FutureExt;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///         .on("test", |payload: Payload, socket: Client| {
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
        self.socket
            .read()
            .await
            .emit(&self.nsp, event.into(), data.into())
            .await
    }

    /// Disconnects this client from the server by sending a `socket.io` closing
    /// packet.
    /// # Example
    /// ```rust
    /// use rust_socketio::{asynchronous::{ClientBuilder, Client}, Payload};
    /// use serde_json::json;
    /// use futures_util::{FutureExt, future::BoxFuture};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     // apparently the syntax for functions is a bit verbose as rust currently doesn't
    ///     // support an `AsyncFnMut` type that conform with async functions
    ///     fn handle_test(payload: Payload, socket: Client) -> BoxFuture<'static, ()> {
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
        self.manually_disconnected.store(true, Ordering::Release);

        let disconnect_packet =
            Packet::new(PacketId::Disconnect, self.nsp.clone(), None, None, 0, None);

        self.socket.read().await.send(disconnect_packet).await?;
        self.socket.read().await.disconnect().await?;

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
    ///         .on("foo", |payload: Payload, _| async move { println!("Received: {:#?}", payload) }.boxed())
    ///         .connect()
    ///         .await
    ///         .expect("connection failed");
    ///
    ///     let ack_callback = |message: Payload, socket: Client| {
    ///         async move {
    ///             match message {
    ///                 Payload::Text(values) => println!("{:#?}", values),
    ///                 Payload::Binary(bytes) => println!("Received bytes: {:#?}", bytes),
    ///                 // This is deprecated use Payload::Text instead
    ///                 Payload::String(str) => println!("{}", str),
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
        F: for<'a> std::ops::FnMut(Payload, Client) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let id = thread_rng().gen_range(0..999);
        let socket_packet =
            Packet::new_from_payload(data.into(), event.into(), &self.nsp, Some(id))?;

        let ack = Ack {
            id,
            time_started: Instant::now(),
            timeout,
            callback: Callback::<DynAsyncCallback>::new(callback),
        };

        // add the ack to the tuple of outstanding acks
        self.outstanding_acks.write().await.push(ack);

        self.socket.read().await.send(socket_packet).await
    }

    async fn callback<P: Into<Payload>>(&self, event: &Event, payload: P) -> Result<()> {
        let mut builder = self.builder.write().await;
        let payload = payload.into();

        if let Some(callback) = builder.on.get_mut(event) {
            callback(payload.clone(), self.clone()).await;
        }

        // Call on_any for all common and custom events.
        match event {
            Event::Message | Event::Custom(_) => {
                if let Some(callback) = builder.on_any.as_mut() {
                    callback(event.clone(), payload, self.clone()).await;
                }
            }
            _ => (),
        }

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
                                Payload::from(payload.to_owned()),
                                self.clone(),
                            )
                            .await;
                        }
                        if let Some(ref attachments) = socket_packet.attachments {
                            if let Some(payload) = attachments.get(0) {
                                ack.callback.deref_mut()(
                                    Payload::Binary(payload.to_owned()),
                                    self.clone(),
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
                self.callback(&event, Payload::Binary(binary_payload.to_owned()))
                    .await?;
            }
        }
        Ok(())
    }

    /// A method that parses a packet and eventually calls the corresponding
    /// callback with the supplied data.
    async fn handle_event(&self, packet: &Packet) -> Result<()> {
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
            self.callback(&event, data.to_string()).await?;
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
                        self.callback(&Event::Error, err.to_string()).await?;
                        return Err(err);
                    }
                }
                PacketId::BinaryEvent => {
                    if let Err(err) = self.handle_binary_event(packet).await {
                        self.callback(&Event::Error, err.to_string()).await?;
                    }
                }
                PacketId::Connect => {
                    self.server_disconnected.store(false, Ordering::Release);
                    self.callback(&Event::Connect, "").await?;
                }
                PacketId::Disconnect => {
                    self.server_disconnected.store(true, Ordering::Release);
                    self.callback(&Event::Close, "").await?;
                }
                PacketId::ConnectError => {
                    self.callback(
                        &Event::Error,
                        String::from("Received an ConnectError frame: ")
                            + packet
                                .data
                                .as_ref()
                                .unwrap_or(&String::from("\"No error message provided\"")),
                    )
                    .await?;
                }
                PacketId::Event => {
                    if let Err(err) = self.handle_event(packet).await {
                        self.callback(&Event::Error, err.to_string()).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Indicates whether the client should try to reconnect
    pub(crate) async fn should_reconnect(&self) -> bool {
        let manually_disconnected = self.manually_disconnected.load(Ordering::Acquire);
        let server_disconnected = self.server_disconnected.load(Ordering::Acquire);

        if server_disconnected {
            self.builder.read().await.reconnect_on_disconnect
        } else {
            !manually_disconnected
        }
    }

    /// Returns the packet stream for the client.
    pub(crate) fn as_stream<'a>(
        &'a self,
    ) -> Pin<Box<dyn Stream<Item = Result<Packet>> + Send + 'a>> {
        let socket_clone = self.socket.clone();

        stream::unfold(socket_clone, |socket| async {
            let mut socket_read = {
                let s = socket.read().await;
                (*s).clone()
            };
            // wait for the next payload
            let packet: Option<std::result::Result<Packet, Error>> = socket_read.next().await;
            match packet {
                // end the stream if the underlying one is closed
                None => None,
                Some(Err(err)) => {
                    // call the error callback
                    match self.callback(&Event::Error, err.to_string()).await {
                        Err(callback_err) => Some((Err(callback_err), socket)),
                        Ok(_) => Some((Err(err), socket)),
                    }
                }
                Some(Ok(packet)) => match self.handle_socketio_packet(&packet).await {
                    Err(callback_err) => Some((Err(callback_err), socket)),
                    Ok(_) => Some((Ok(packet), socket)),
                },
            }
        })
        .boxed()
    }
}

#[cfg(test)]
mod test {

    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::Duration,
    };

    use bytes::Bytes;
    use futures_util::{FutureExt, StreamExt};
    use native_tls::TlsConnector;
    use serde_json::json;
    use tokio::{
        sync::mpsc,
        time::{sleep, timeout},
    };

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
            .on("test", |msg, _| {
                async {
                    match msg {
                        Payload::Text(values) => println!("Received json: {:#?}", values),
                        #[allow(deprecated)]
                        Payload::String(str) => println!("Received string: {}", str),
                        Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
                    }
                }
                .boxed()
            })
            .connect()
            .await?;

        let payload = json!({"token": 123_i32});
        let result = socket.emit("test", Payload::from(payload.clone())).await;

        assert!(result.is_ok());

        let ack = socket
            .emit_with_ack(
                "test",
                Payload::from(payload),
                Duration::from_secs(1),
                |message: Payload, socket: Client| {
                    async move {
                        let result = socket
                            .emit("test", Payload::from(json!({"got ack": true})))
                            .await;
                        assert!(result.is_ok());

                        println!("Yehaa! My ack got acked?");
                        if let Payload::Text(json) = message {
                            println!("Received json Ack");
                            println!("Ack data: {:#?}", json);
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
    async fn socket_io_async_callback() -> Result<()> {
        // Test whether asynchronous callbacks are fully executed.
        let url = crate::test::socket_io_server();

        // This synchronization mechanism is used to let the test know that the end of the
        // async callback was reached.
        let notify = Arc::new(tokio::sync::Notify::new());
        let notify_clone = notify.clone();

        let socket = ClientBuilder::new(url)
            .on("test", move |_, _| {
                let cl = notify_clone.clone();
                async move {
                    sleep(Duration::from_secs(1)).await;
                    // The async callback should be awaited and not aborted.
                    // Thus, the notification should be called.
                    cl.notify_one();
                }
                .boxed()
            })
            .connect()
            .await?;

        let payload = json!({"token": 123_i32});
        let result = socket.emit("test", Payload::from(payload)).await;

        assert!(result.is_ok());
        // If the timeout did not trigger, the async callback was fully executed.
        let timeout = timeout(Duration::from_secs(5), notify.notified()).await;
        assert!(timeout.is_ok());

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
            .on("test", |str, _| {
                async move { println!("Received: {:#?}", str) }.boxed()
            })
            .on("message", |payload, _| {
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
                |payload, _| async move {
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
    async fn socket_io_reconnect_integration() -> Result<()> {
        static CONNECT_NUM: AtomicUsize = AtomicUsize::new(0);
        static MESSAGE_NUM: AtomicUsize = AtomicUsize::new(0);

        let url = crate::test::socket_io_restart_server();

        let socket = ClientBuilder::new(url)
            .reconnect(true)
            .max_reconnect_attempts(100)
            .reconnect_delay(100, 100)
            .on("open", |_, socket| {
                async move {
                    CONNECT_NUM.fetch_add(1, Ordering::Release);
                    let r = socket.emit_with_ack(
                        "message",
                        json!(""),
                        Duration::from_millis(100),
                        |_, _| async move {}.boxed(),
                    );
                    assert!(r.await.is_ok(), "should emit message success");
                }
                .boxed()
            })
            .on("message", |_, _socket| {
                async move {
                    // test the iterator implementation and make sure there is a constant
                    // stream of packets, even when reconnecting
                    MESSAGE_NUM.fetch_add(1, Ordering::Release);
                }
                .boxed()
            })
            .connect()
            .await;

        assert!(socket.is_ok(), "should connect success");
        let socket = socket.unwrap();

        // waiting for server to emit message
        sleep(Duration::from_millis(500)).await;

        assert_eq!(load(&CONNECT_NUM), 1, "should connect once");
        assert_eq!(load(&MESSAGE_NUM), 1, "should receive one");

        let r = socket.emit("restart_server", json!("")).await;
        assert!(r.is_ok(), "should emit restart success");

        // waiting for server to restart
        for _ in 0..10 {
            sleep(Duration::from_millis(400)).await;
            if load(&CONNECT_NUM) == 2 && load(&MESSAGE_NUM) == 2 {
                break;
            }
        }

        assert_eq!(load(&CONNECT_NUM), 2, "should connect twice");
        assert_eq!(load(&MESSAGE_NUM), 2, "should receive two messages");

        socket.disconnect().await?;
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
            .on("test", |str, _| {
                async move { println!("Received: {:#?}", str) }.boxed()
            })
            .on("message", |payload, _| {
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
                |payload, _| async move {
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
    async fn socket_io_on_any_integration() -> Result<()> {
        let url = crate::test::socket_io_server();

        let (tx, mut rx) = mpsc::channel(2);

        let mut _socket = ClientBuilder::new(url)
            .namespace("/")
            .auth(json!({ "password": "123" }))
            .on_any(move |event, payload, _| {
                let clone_tx = tx.clone();
                async move {
                    if let Payload::Text(values) = payload {
                        println!("{event}: {values:#?}");
                    }
                    clone_tx.send(String::from(event)).await.unwrap();
                }
                .boxed()
            })
            .connect()
            .await?;

        let event = rx.recv().await.unwrap();
        assert_eq!(event, "message");

        let event = rx.recv().await.unwrap();
        assert_eq!(event, "test");

        Ok(())
    }

    #[tokio::test]
    async fn socket_io_auth_builder_integration() -> Result<()> {
        let url = crate::test::socket_io_auth_server();
        let nsp = String::from("/admin");
        let socket = ClientBuilder::new(url)
            .namespace(nsp.clone())
            .auth(json!({ "password": "123" }))
            .connect_manual()
            .await?;

        // open packet
        let mut socket_stream = socket.as_stream();
        let _ = socket_stream.next().await.unwrap()?;

        println!("Here12");
        let packet = socket_stream.next().await.unwrap()?;
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

    async fn test_socketio_socket(socket: Client, nsp: String) -> Result<()> {
        // open packet
        let mut socket_stream = socket.as_stream();
        let _: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

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

        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

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
        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

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

        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

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

        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

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

        let cb = |message: Payload, _| {
            async {
                println!("Yehaa! My ack got acked?");
                if let Payload::Text(values) = message {
                    println!("Received json ack");
                    println!("Ack data: {:#?}", values);
                }
            }
            .boxed()
        };

        assert!(socket
            .emit_with_ack(
                "test",
                Payload::from("123".to_owned()),
                Duration::from_secs(10),
                cb
            )
            .await
            .is_ok());

        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

        assert!(packet.is_some());
        let packet = packet.unwrap();
        assert_eq!(
            packet,
            Packet::new(
                PacketId::Event,
                nsp.clone(),
                Some("[\"test-received\",123]".to_owned()),
                None,
                0,
                None,
            )
        );

        let packet: Option<Packet> = Some(socket_stream.next().await.unwrap()?);

        assert!(packet.is_some());
        let packet = packet.unwrap();
        assert!(matches!(
            packet,
            Packet {
                packet_type: PacketId::Ack,
                nsp: _,
                data: Some(_),
                id: Some(_),
                attachment_count: 0,
                attachments: None,
            }
        ));

        Ok(())
    }

    fn load(num: &AtomicUsize) -> usize {
        num.load(Ordering::Acquire)
    }
}
