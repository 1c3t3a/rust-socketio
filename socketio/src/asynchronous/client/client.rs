use std::sync::Arc;

use tokio::sync::Mutex;

use futures_util::{future::BoxFuture, StreamExt};

use tokio::{runtime::Handle, sync::RwLock, time::Duration};

use super::RawClient;
use crate::{
    asynchronous::ClientBuilder,
    error::{Error, Result},
    packet::{Packet, PacketId},
    Event, Payload,
};

use backoff::ExponentialBackoff;
use backoff::{backoff::Backoff, ExponentialBackoffBuilder};

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the client will connect to the default namespace `"/"`.
#[derive(Clone)]
pub struct Client {
    builder: Arc<Mutex<ClientBuilder>>,
    client: Arc<RwLock<RawClient>>,
    backoff: ExponentialBackoff,
}

impl Client {
    /// Creates a socket with a certain address to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    /// ```
    pub(crate) async fn new(builder: ClientBuilder) -> Result<Self> {
        let builder_clone = builder.clone();
        let client = builder_clone.connect_raw().await?;
        let backoff = ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_millis(builder.reconnect_delay_min))
            .with_max_interval(Duration::from_millis(builder.reconnect_delay_max))
            .build();

        let s = Self {
            builder: Arc::new(Mutex::new(builder)),
            client: Arc::new(RwLock::new(client)),
            backoff,
        };
        s.poll_callback();

        Ok(s)
    }

    /// Updates the URL the client will connect to when reconnecting.
    /// This is especially useful for updating query parameters.
    pub fn set_reconnect_url<T: Into<String> + Send + 'static>(&self, address: T) -> Result<()> {
        // Create a channel to signal the completion of the async block
        let (tx, rx) = std::sync::mpsc::channel();
        let builder_clone = self.builder.clone();
        
        // Spawn a new thread to run the async code block
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("Unable to create a new runtime");
            rt.block_on(async {
                let mut builder = builder_clone.lock().await;
                builder.address = address.into();
                tx.send(()).expect("Unable to send completion signal");
            });
        });

        // Wait for the async block to complete
        rx.recv().expect("Unable to receive completion signal");
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
        let client = self.client.read().await;

        // TODO(#230): like js client, buffer emit, resend after reconnect
        client.emit(event.into(), data.into()).await
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
        F: for<'a> std::ops::FnMut(Payload, RawClient) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let client = self.client.read().await;
        // TODO(#230): like js client, buffer emit, resend after reconnect
        client.emit_with_ack(event, data, timeout, callback).await
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
        let client = self.client.read().await;
        client.disconnect().await
    }

    async fn reconnect(&mut self) -> Result<()> {
        let mut reconnect_attempts = 0;
        let (reconnect, max_reconnect_attempts) = {
            let builder = self.builder.lock().await;
            (builder.reconnect, builder.max_reconnect_attempts)
        };

        if reconnect {
            loop {
                if let Some(max_reconnect_attempts) = max_reconnect_attempts {
                    reconnect_attempts += 1;
                    if reconnect_attempts > max_reconnect_attempts {
                        break;
                    }
                }

                if let Some(backoff) = self.backoff.next_backoff() {
                    std::thread::sleep(backoff);
                }

                if self.do_reconnect().await.is_ok() {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn do_reconnect(&self) -> Result<()> {
        let builder = self.builder.lock().await;
        let new_client = builder.clone().connect_raw().await?;
        let mut client = self.client.write().await;
        *client = new_client;

        Ok(())
    }

    fn poll_callback(&self) {
        let mut self_clone = self.clone();
        tokio::spawn(async move {
            loop {
                let should_reconnect;
                {
                    let client_lock = self_clone.client.read().await;
                    let mut stream = client_lock.as_stream(); // Assume this method returns a Stream
                    let packet = stream.next().await;

                    should_reconnect = match packet {
                        Some(Err(Error::IncompleteResponseFromEngineIo(_))) => {
                            // TODO: Log the error
                            true
                        }
                        Some(Ok(Packet {
                            packet_type: PacketId::Disconnect,
                            ..
                        })) => {
                            let lock = self_clone.builder.lock().await;
                            lock.reconnect_on_disconnect
                        }
                        _ => false,
                    };
                }

                if should_reconnect {
                    let _ = self_clone.disconnect().await;
                    let _ = self_clone.reconnect().await;
                }
            }
        });
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::UNIX_EPOCH,
    };

    use super::ClientBuilder;
    use super::*;
    use crate::error::Result;
    use futures_util::FutureExt;
    use serde_json::json;
    use std::time::{Duration, SystemTime};
    use url::Url;

    #[tokio::test]
    async fn socket_io_reconnect_integration() -> Result<()> {
        static CONNECT_NUM: AtomicUsize = AtomicUsize::new(0);
        static CLOSE_NUM: AtomicUsize = AtomicUsize::new(0);
        static MESSAGE_NUM: AtomicUsize = AtomicUsize::new(0);

        let url = crate::test::socket_io_restart_server();

        let socket = ClientBuilder::new(url)
            .reconnect(true)
            .max_reconnect_attempts(100)
            .reconnect_delay(100, 100)
            .on(Event::Connect, |_, socket| {
                async move {
                    CONNECT_NUM.fetch_add(1, Ordering::Release);
                    let r = socket.emit_with_ack(
                        "message",
                        json!(""),
                        Duration::from_millis(100),
                        |_, _| async {}.boxed(),
                    );
                    assert!(r.await.is_ok(), "should emit message success");
                }
                .boxed()
            })
            .on(Event::Close, move |_, _| {
                async move {
                    CLOSE_NUM.fetch_add(1, Ordering::Release);
                }
                .boxed()
            })
            .on("message", move |_, _socket| {
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
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert_eq!(load(&CONNECT_NUM), 1, "should connect once");
        assert_eq!(load(&MESSAGE_NUM), 1, "should receive one");
        assert_eq!(load(&CLOSE_NUM), 0, "should not close");

        let r = socket.emit("restart_server", json!("")).await;
        assert!(r.is_ok(), "should emit restart success");

        // waiting for server to restart
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if load(&CONNECT_NUM) == 2 && load(&MESSAGE_NUM) == 2 {
                break;
            }
        }

        assert_eq!(load(&CONNECT_NUM), 2, "should connect twice");
        assert_eq!(load(&MESSAGE_NUM), 2, "should receive two messages");
        assert_eq!(load(&CLOSE_NUM), 1, "should close once");

        socket.disconnect().await?;
        Ok(())
    }

    #[tokio::test]
    async fn socket_io_reconnect_url_auth_integration() -> Result<()> {
        static CONNECT_NUM: AtomicUsize = AtomicUsize::new(0);
        static CLOSE_NUM: AtomicUsize = AtomicUsize::new(0);
        static MESSAGE_NUM: AtomicUsize = AtomicUsize::new(0);

        fn get_url() -> Url {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis();
            let mut url = crate::test::socket_io_restart_url_auth_server();
            url.set_query(Some(&format!("timestamp={timestamp}")));
            url
        }

        let socket = ClientBuilder::new(get_url())
            .reconnect(true)
            .max_reconnect_attempts(100)
            .reconnect_delay(100, 100)
            .on(Event::Connect, move |_, socket| {
                async move {
                    CONNECT_NUM.fetch_add(1, Ordering::Release);
                    let result = socket
                        .emit_with_ack("message", json!(""), Duration::from_millis(100), |_, _| {
                            async {}.boxed()
                        })
                        .await;
                    assert!(result.is_ok(), "should emit message success");
                }
                .boxed()
            })
            .on(Event::Close, move |_, _| {
                async move {
                    CLOSE_NUM.fetch_add(1, Ordering::Release);
                }
                .boxed()
            })
            .on("message", move |_, _| {
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
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert_eq!(load(&CONNECT_NUM), 1, "should connect once");
        assert_eq!(load(&MESSAGE_NUM), 1, "should receive one");
        assert_eq!(load(&CLOSE_NUM), 0, "should not close");

        // waiting for timestamp in url to expire
        std::thread::sleep(std::time::Duration::from_secs(1));

        socket.set_reconnect_url(get_url())?;

        let result = socket.emit("restart_server", json!("")).await;
        assert!(result.is_ok(), "should emit restart success");

        // waiting for server to restart
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if load(&CONNECT_NUM) == 2 && load(&MESSAGE_NUM) == 2 {
                break;
            }
        }

        assert_eq!(load(&CONNECT_NUM), 2, "should connect twice");
        assert_eq!(load(&MESSAGE_NUM), 2, "should receive two messages");
        assert_eq!(load(&CLOSE_NUM), 1, "should close once");

        socket.disconnect().await?;
        Ok(())
    }

    fn load(num: &AtomicUsize) -> usize {
        num.load(Ordering::Acquire)
    }
}
