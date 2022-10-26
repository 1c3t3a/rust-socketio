use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use super::{ClientBuilder, RawClient};
use crate::{error::Result, packet::Packet, Error};
pub(crate) use crate::{event::Event, payload::Payload};
use backoff::ExponentialBackoff;
use backoff::{backoff::Backoff, ExponentialBackoffBuilder};

#[derive(Clone)]
pub struct Client {
    builder: ClientBuilder,
    client: Arc<RwLock<RawClient>>,
    backoff: ExponentialBackoff,
}

impl Client {
    pub(crate) fn new(builder: ClientBuilder) -> Result<Self> {
        let builder_clone = builder.clone();
        let client = builder_clone.connect_raw()?;
        let backoff = ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_millis(builder.reconnect_delay_min))
            .with_max_interval(Duration::from_millis(builder.reconnect_delay_max))
            .build();

        let s = Self {
            builder,
            client: Arc::new(RwLock::new(client)),
            backoff,
        };
        s.poll_callback();

        Ok(s)
    }

    /// Sends a message to the server using the underlying `engine.io` protocol.
    /// This message takes an event, which could either be one of the common
    /// events like "message" or "error" or a custom event like "foo". But be
    /// careful, the data string needs to be valid JSON. It's recommended to use
    /// a library like `serde_json` to serialize the data properly.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::{ClientBuilder, RawClient, Payload};
    /// use serde_json::json;
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("test", |payload: Payload, socket: RawClient| {
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
    pub fn emit<E, D>(&self, event: E, data: D) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        let client = self.client.read()?;
        // TODO(#230): like js client, buffer emit, resend after reconnect
        client.emit(event, data)
    }

    /// Sends a message to the server but `alloc`s an `ack` to check whether the
    /// server responded in a given time span. This message takes an event, which
    /// could either be one of the common events like "message" or "error" or a
    /// custom event like "foo", as well as a data parameter.
    /// It requires a timeout `Duration` in which the client needs to answer.
    /// If the ack is acked in the correct time span, the specified callback is
    /// called. The callback consumes a [`Payload`] which represents the data send
    /// by the server.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::{ClientBuilder, Payload, RawClient};
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use std::thread::sleep;
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
    ///     .on("foo", |payload: Payload, _| println!("Received: {:#?}", payload))
    ///     .connect()
    ///     .expect("connection failed");
    ///
    /// let ack_callback = |message: Payload, socket: RawClient| {
    ///     match message {
    ///         Payload::Json(data) => println!("{:?}", data),
    ///         Payload::Binary(bytes) => println!("Received bytes: {:#?}", bytes),
    ///         Payload::Multi(vec) => println!("Received multi: {:?}", vec),
    ///         _ => {}
    ///    }
    /// };
    ///
    /// let payload = json!({"token": 123});
    /// socket.emit_with_ack("foo", payload, Duration::from_secs(2), ack_callback).unwrap();
    ///
    /// sleep(Duration::from_secs(2));
    /// ```
    pub fn emit_with_ack<F, E, D>(
        &self,
        event: E,
        data: D,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: for<'a> FnMut(Payload, RawClient) + 'static + Sync + Send,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let client = self.client.read()?;
        // TODO(#230): like js client, buffer emit, resend after reconnect
        client.emit_with_ack(event, data, timeout, callback)
    }

    /// Disconnects this client from the server by sending a `socket.io` closing
    /// packet.
    /// # Example
    /// ```rust
    /// use rust_socketio::{ClientBuilder, Payload, RawClient};
    /// use serde_json::json;
    ///
    /// fn handle_test(payload: Payload, socket: RawClient) {
    ///     println!("Received: {:#?}", payload);
    ///     socket.emit("test", json!({"hello": true})).expect("Server unreachable");
    /// }
    ///
    /// let mut socket = ClientBuilder::new("http://localhost:4200/")
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
        let client = self.client.read()?;
        client.disconnect()
    }

    fn reconnect(&mut self) {
        let mut reconnect_attempts = 0;
        if self.builder.reconnect {
            loop {
                if let Some(max_reconnect_attempts) = self.builder.max_reconnect_attempts {
                    if reconnect_attempts > max_reconnect_attempts {
                        break;
                    }
                }
                reconnect_attempts += 1;

                if let Some(backoff) = self.backoff.next_backoff() {
                    std::thread::sleep(backoff);
                }

                if self.do_reconnect().is_ok() {
                    break;
                }
            }
        }
    }

    fn do_reconnect(&self) -> Result<()> {
        let new_client = self.builder.clone().connect_raw()?;
        let mut client = self.client.write()?;
        *client = new_client;

        Ok(())
    }

    pub(crate) fn iter(&self) -> Iter {
        Iter {
            socket: self.client.clone(),
        }
    }

    fn poll_callback(&self) {
        let mut self_clone = self.clone();
        // Use thread to consume items in iterator in order to call callbacks
        std::thread::spawn(move || {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            for packet in self_clone.iter() {
                if let _e @ Err(Error::IncompleteResponseFromEngineIo(_)) = packet {
                    //TODO: 0.3.X handle errors
                    //TODO: logging error
                    let _ = self_clone.disconnect();
                    self_clone.reconnect();
                }
            }
        });
    }
}

pub(crate) struct Iter {
    socket: Arc<RwLock<RawClient>>,
}

impl Iterator for Iter {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        let socket = self.socket.read();
        match socket {
            Ok(socket) => match socket.poll() {
                Err(err) => Some(Err(err)),
                Ok(Some(packet)) => Some(Ok(packet)),
                // If the underlying engineIO connection is closed,
                // throw an error so we know to reconnect
                Ok(None) => Some(Err(Error::StoppedEngineIoSocket)),
            },
            Err(_) => {
                // Lock is poisoned, our iterator is useless.
                None
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::error::Result;
    use crate::ClientBuilder;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn socket_io_reconnect_integration() -> Result<()> {
        let url = crate::test::socket_io_restart_server();

        let connect_num = Arc::new(AtomicUsize::new(0));
        let close_num = Arc::new(AtomicUsize::new(0));
        let message_num = Arc::new(AtomicUsize::new(0));

        let connect_num_clone = Arc::clone(&connect_num);
        let close_num_clone = Arc::clone(&close_num);
        let message_num_clone = Arc::clone(&message_num);

        let socket = ClientBuilder::new(url)
            .reconnect(true)
            .max_reconnect_attempts(100)
            .reconnect_delay(100, 100)
            .on(Event::Connect, move |_, socket| {
                connect_num_clone.fetch_add(1, Ordering::SeqCst);
                let r = socket.emit_with_ack(
                    "message",
                    json!(""),
                    Duration::from_millis(100),
                    |_, _| {},
                );
                assert!(r.is_ok(), "should emit message success");
            })
            .on(Event::Close, move |_, _| {
                close_num_clone.fetch_add(1, Ordering::SeqCst);
            })
            .on("message", move |_, _socket| {
                // test the iterator implementation and make sure there is a constant
                // stream of packets, even when reconnecting
                message_num_clone.fetch_add(1, Ordering::SeqCst);
            })
            .connect();

        assert!(socket.is_ok(), "should connect success");
        let socket = socket.unwrap();

        // waiting for server to emit message
        std::thread::sleep(std::time::Duration::from_millis(500));

        assert_eq!(load(&connect_num), 1, "should connect once");
        assert_eq!(load(&message_num), 1, "should receive one");
        assert_eq!(load(&close_num), 0, "should not close");

        let r = socket.emit("restart_server", json!(""));
        assert!(r.is_ok(), "should emit restart success");

        // waiting for server to restart
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            if load(&connect_num) == 2 && load(&message_num) == 2 {
                break;
            }
        }

        assert_eq!(load(&connect_num), 2, "should connect twice");
        assert_eq!(load(&message_num), 2, "should receive two messages");
        assert_eq!(load(&close_num), 1, "should close once");

        socket.disconnect()?;
        Ok(())
    }

    #[test]
    fn socket_io_iterator_integration() -> Result<()> {
        let url = crate::test::socket_io_server();
        let builder = ClientBuilder::new(url);
        let builder_clone = builder.clone();

        let client = Arc::new(RwLock::new(builder_clone.connect_raw()?));
        let mut socket = Client {
            builder,
            client,
            backoff: Default::default(),
        };
        let socket_clone = socket.clone();

        let packets: Arc<RwLock<Vec<Packet>>> = Default::default();
        let packets_clone = packets.clone();

        std::thread::spawn(move || {
            for packet in socket_clone.iter() {
                {
                    let mut packets = packets_clone.write().unwrap();
                    if let Ok(packet) = packet {
                        (*packets).push(packet);
                    }
                }
            }
        });

        // waiting for client to emit messages
        std::thread::sleep(Duration::from_millis(100));
        let lock = packets.read().unwrap();
        let pre_num = lock.len();
        drop(lock);

        let _ = socket.disconnect();
        socket.reconnect();

        // waiting for client to emit messages
        std::thread::sleep(Duration::from_millis(100));

        let lock = packets.read().unwrap();
        let post_num = lock.len();
        drop(lock);

        assert!(
            pre_num < post_num,
            "pre_num {} should less than post_num {}",
            pre_num,
            post_num
        );

        Ok(())
    }

    fn load(num: &Arc<AtomicUsize>) -> usize {
        num.load(Ordering::SeqCst)
    }
}
