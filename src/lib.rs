//! Rust socket.io is a socket.io client written in the Rust Programming Language.
//! ## Example usage
//!
//! ``` rust
//! use rust_socketio::Socket;
//! use serde_json::json;
//! use std::thread::sleep;
//! use std::time::Duration;
//!
//! let mut socket = Socket::new(String::from("http://localhost:4200"), Some("/admin"));
//!
//! // callback for the "foo" event
//! socket.on("foo", |message| println!("{}", message)).unwrap();
//!
//! // connect to the server
//! socket.connect().expect("Connection failed");
//!
//! // emit to the "foo" event
//! let payload = json!({"token": 123});
//! socket.emit("foo", &payload.to_string()).expect("Server unreachable");
//!
//! // define a callback, that's executed when the ack got acked
//! let ack_callback = |message: String| {
//!     println!("Yehaa! My ack got acked?");
//!     println!("Ack data: {}", message);
//! };
//!
//! sleep(Duration::from_secs(2));
//!
//! // emit with an ack
//! let ack = socket
//!     .emit_with_ack("test", &payload.to_string(), Duration::from_secs(2), ack_callback)
//!     .expect("Server unreachable");
//! ```
//!
//! ## Current features
//!
//! This version of the client lacks some features that the reference client
//! would provide. The underlying `engine.io` protocol still uses long-polling
//! instead of websockets. This will be resolved as soon as both the reqwest
//! libary as well as `tungsenite-websockets` will bump their `tokio` version to
//! 1.0.0. At the moment only `reqwest` is used for long-polling. The full
//! `engine-io` protocol is implemented and most of the features concerning the
//! 'normal' `socket.io` protocol are working.
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
//!
//! The whole crate is written in asynchronous Rust and it's necessary to use
//! [tokio](https://docs.rs/tokio/1.0.1/tokio/), or other executors with this
//! library to resolve the futures.
//!
#![allow(clippy::rc_buffer)]
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

use crate::error::Error;
use std::time::Duration;

use crate::socketio::transport::TransportClient;

/// A socket which handles communication with the server. It's initialized with
/// a specific address as well as an optional namespace to connect to. If `None`
/// is given the server will connect to the default namespace `"/"`.
#[derive(Debug)]
pub struct Socket {
    /// The inner transport client to delegate the methods to.
    transport: TransportClient,
}

impl Socket {
    /// Creates a socket with a certain adress to connect to as well as a
    /// namespace. If `None` is passed in as namespace, the default namespace
    /// `"/"` is taken.
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::Socket;
    ///
    /// // this connects the socket to the given url as well as the default
    /// // namespace "/"
    /// let socket = Socket::new("http://localhost:80", None);
    ///
    /// // this connects the socket to the given url as well as the namespace
    /// // "/admin"
    /// let socket = Socket::new("http://localhost:80", Some("/admin"));
    /// ```
    pub fn new<T: Into<String>>(address: T, namespace: Option<&str>) -> Self {
        Socket {
            transport: TransportClient::new(address, namespace.map(String::from)),
        }
    }

    /// Registers a new callback for a certain event. This returns an
    /// `Error::IllegalActionAfterOpen` error if the callback is registered
    /// after a call to the `connect` method.
    /// # Example
    /// ```rust
    /// use rust_socketio::Socket;
    ///
    /// let mut socket = Socket::new("http://localhost:4200", None);
    /// let result = socket.on("foo", |message| println!("{}", message));
    ///
    /// assert!(result.is_ok());
    /// ```
    pub fn on<F>(&mut self, event: &str, callback: F) -> Result<(), Error>
    where
        F: FnMut(String) + 'static + Sync + Send,
    {
        self.transport.on(event.into(), callback)
    }

    /// Connects the client to a server. Afterwards the `emit_*` methods can be
    /// called to interact with the server. Attention: it's not allowed to add a
    /// callback after a call to this method.
    ///
    /// # Example
    /// ```rust
    /// use rust_socketio::Socket;
    ///
    /// let mut socket = Socket::new("http://localhost:4200", None);
    ///
    /// socket.on("foo", |message| println!("{}", message)).unwrap();
    /// let result = socket.connect();
    ///
    /// assert!(result.is_ok());
    /// ```
    pub fn connect(&mut self) -> Result<(), Error> {
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
    /// use rust_socketio::Socket;
    /// use serde_json::json;
    ///
    /// let mut socket = Socket::new("http://localhost:4200", None);
    ///
    /// socket.on("foo", |message| println!("{}", message)).unwrap();
    /// socket.connect().expect("Connection failed");
    ///
    /// let payload = json!({"token": 123});
    /// let result = socket.emit("foo", &payload.to_string());
    ///
    /// assert!(result.is_ok());
    /// ```
    #[inline]
    pub fn emit(&mut self, event: &str, data: &str) -> Result<(), Error> {
        self.transport.emit(event.into(), data)
    }

    /// Sends a message to the server but `alloc`s an `ack` to check whether the
    /// server responded in a given timespan. This message takes an event, which
    /// could either be one of the common events like "message" or "error" or a
    /// custom event like "foo", as well as a data parameter. But be careful,
    /// the string needs to be valid JSON. It's even recommended to use a
    /// library like serde_json to serialize the data properly. It also requires
    /// a timeout `Duration` in which the client needs to answer. This method
    /// returns an `Arc<RwLock<Ack>>`. The `Ack` type holds information about
    /// the `ack` system call, such whether the `ack` got acked fast enough and
    /// potential data. It is safe to unwrap the data after the `ack` got acked
    /// from the server. This uses an `RwLock` to reach shared mutability, which
    /// is needed as the server sets the data on the ack later.
    ///
    /// # Example
    /// ```
    /// use rust_socketio::Socket;
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use std::thread::sleep;
    ///
    /// let mut socket = Socket::new("http://localhost:4200", None);
    ///
    /// socket.on("foo", |message| println!("{}", message)).unwrap();
    /// socket.connect().expect("Connection failed");
    ///
    /// let payload = json!({"token": 123});
    /// let ack_callback = |message| { println!("{}", message) };
    ///
    /// socket.emit_with_ack("foo", &payload.to_string(),
    /// Duration::from_secs(2), ack_callback).unwrap();
    ///
    /// sleep(Duration::from_secs(2));
    /// ```
    #[inline]
    pub fn emit_with_ack<F>(
        &mut self,
        event: &str,
        data: &str,
        timeout: Duration,
        callback: F,
    ) -> Result<(), Error>
    where
        F: FnMut(String) + 'static + Send + Sync,
    {
        self.transport
            .emit_with_ack(event.into(), data, timeout, callback)
    }
}

#[cfg(test)]
mod test {

    use std::thread::sleep;

    use super::*;
    use serde_json::json;
    const SERVER_URL: &str = "http://localhost:4200";

    #[test]
    fn it_works() {
        let mut socket = Socket::new(SERVER_URL, None);

        let result = socket.on("test", |msg| println!("{}", msg));
        assert!(result.is_ok());

        let result = socket.connect();
        assert!(result.is_ok());

        let payload = json!({"token": 123});
        let result = socket.emit("test", &payload.to_string());

        assert!(result.is_ok());

        let mut socket_clone = socket.clone();
        let ack_callback = move |message: String| {
            let result = socket_clone.emit("test", &json!({"got ack": true}).to_string());
            assert!(result.is_ok());

            println!("Yehaa! My ack got acked?");
            println!("Ack data: {}", message);
        };

        let ack = socket.emit_with_ack(
            "test",
            &payload.to_string(),
            Duration::from_secs(2),
            ack_callback,
        );
        assert!(ack.is_ok());

        sleep(Duration::from_secs(2));
    }
}
