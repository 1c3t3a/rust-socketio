//! Rust socket.io is a socket-io client for the rust programming language.
//! ## Example usage
//!
//! ``` rust
//! use rust_socketio::Socket;
//! use serde_json::json;
//! use tokio::time::sleep;
//!
//! #[tokio::main]
//! async fn main() {
//!     let mut socket = Socket::new(String::from("http://localhost:80"), Some("/admin"));
//!
//!     // callback for the "foo" event
//!     socket.on("foo", |message| println!("{}", message)).unwrap();
//!
//!     // connect to the server
//!     socket.connect().await.expect("Connection failed");
//!
//!     // emit to the "foo" event
//!     let payload = json!({"token": 123});
//!     socket.emit("foo", payload.to_string()).await.expect("Server unreachable");
//!
//!     // emit with an ack
//!     let ack = socket.emit_with_ack("foo", payload.to_string(), Duration::from_secs(2)).await.unwrap();
//!
//!     sleep(Duration::from_secs(2)).await;
//!
//!     // check if ack is present and read the data
//!     if ack.read().expect("Server panicked anyway").acked {
//!         println!("{}", ack.read().expect("Server panicked anyway").data.as_ref().unwrap());
//!     }
//! }
//! ```
//!
//! ## Current features
//!
//! This is the first released version of the client, so it still lacks some features that the normal client would provide. First of all the underlying engine.io protocol still uses long-polling instead of websockets. This will be resolved as soon as both the reqwest libary as well as tungsenite-websockets will bump their tokio version to 1.0.0. At the moment only reqwest is used for async long polling. In general the full engine-io protocol is implemented and most of the features concerning the 'normal' socket.io protocol work as well.
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
//! - send json-data to the server (recommended to use serde_json as it provides safe handling of json data).
//! - send json-data to the server and receive an ack with a possible message.
//!
//! What's currently missing is the emitting of binary data - I aim to implement this as soon as possible.
//!
//! The whole crate is written in asynchronous rust and it's necessary to use [tokio](https://docs.rs/tokio/1.0.1/tokio/), or other executors with this libary to resolve the futures.
//!

/// A small macro that spawns a scoped thread.
/// Used for calling the callback functions.
macro_rules! spawn_scoped {
    ($e:expr) => {
        crossbeam_utils::thread::scope(|s| {
            s.spawn(|_| $e);
        })
        .unwrap();
    };
}

mod engineio;
/// Contains the types and the code concerning the
/// socket.io protocol.
pub mod socketio;

/// Contains the error type that will be returned with
/// every result in this crate. Handles all kind of errors.
pub mod error;

use crate::error::Error;
use std::time::Duration;

use crate::socketio::transport::TransportClient;

/// A socket that handles communication with the server.
/// It's initialized with a specific address as well as an
/// optional namespace to connect to. If None is given the
/// server will connect to the default namespace `"/"`.
pub struct Socket {
    transport: TransportClient,
}

impl Socket {
    /// Creates a socket with a certain adress to connect to as well as a namespace. If None is passed in
    /// as namespace, the default namespace "/" is taken.
    /// # Example
    /// ```rust
    /// use rust_socketio::Socket;
    ///
    /// // this connects the socket to the given url as well as the default namespace "/""
    /// let socket = Socket::new(String::from("http://localhost:80"), None);
    ///
    /// // this connects the socket to the given url as well as the namespace "/admin"
    /// let socket = Socket::new(String::from("http://localhost:80"), Some("/admin"));
    /// ```
    pub fn new(address: String, namespace: Option<&str>) -> Self {
        Socket {
            transport: TransportClient::new(address, namespace.map(String::from)),
        }
    }

    /// Registers a new callback for a certain event. This returns an `Error::IllegalActionAfterOpen` error
    /// if the callback is registered after a call to the `connect` method.
    /// # Example
    /// ```rust
    /// use rust_socketio::Socket;
    ///
    /// let mut socket = Socket::new(String::from("http://localhost:80"), None);
    /// let result = socket.on("foo", |message| println!("{}", message));
    ///
    /// assert!(result.is_ok());
    /// ```
    pub fn on<F>(&mut self, event: &str, callback: F) -> Result<(), Error>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        self.transport.on(event.into(), callback)
    }

    /// Connects the client to a server. Afterwards the emit_* methods could be called to inter-
    /// act with the server. Attention: it is not allowed to add a callback after a call to this method.
    /// # Example
    /// ```rust
    /// use rust_socketio::Socket;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = Socket::new(String::from("http://localhost:80"), None);
    ///
    ///     socket.on("foo", |message| println!("{}", message)).unwrap();
    ///     let result = socket.connect().await;
    ///
    ///     assert!(result.is_ok());
    /// }
    /// ```
    pub async fn connect(&mut self) -> Result<(), Error> {
        self.transport.connect().await
    }

    /// Sends a message to the server. This uses the underlying engine.io protocol to do so.
    /// This message takes an  event, which could either be one of the common events like "message" or "error"
    /// or a custom event like "foo". But be careful, the data string needs to be valid json.
    /// It's even recommended to use a libary like serde_json to serialize your data properly.
    /// # Example
    /// ```
    /// use rust_socketio::Socket;
    /// use serde_json::{Value, json};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = Socket::new(String::from("http://localhost:80"), None);
    ///
    ///     socket.on("foo", |message| println!("{}", message)).unwrap();
    ///     socket.connect().await.expect("Connection failed");
    ///
    ///     let payload = json!({"token": 123});
    ///     let result = socket.emit("foo", payload.to_string()).await;
    ///
    ///     assert!(result.is_ok());
    /// }
    /// ```
    pub async fn emit(&mut self, event: &str, data: String) -> Result<(), Error> {
        self.transport.emit(event.into(), data).await
    }

    /// Sends a message to the server but allocs an ack to check whether the server responded in a given timespan.
    /// This message takes an  event, which could either be one of the common events like "message" or "error"
    /// or a custom event like "foo", as well as a data parameter. But be careful, the string needs to be valid json.
    /// It's even recommended to use a libary like serde_json to serialize your data properly.
    /// It also requires a timeout, a `Duration` in which the client needs to answer.
    /// This method returns an `Arc<RwLock<Ack>>`. The `Ack` type holds information about the Ack, which are whether
    /// the ack got acked fast enough and potential data. It is safe to unwrap the data after the ack got acked from the server.
    /// This uses an RwLock to reach shared mutability, which is needed as the server set's the data on the ack later.
    /// # Example
    /// ```
    /// use rust_socketio::Socket;
    /// use serde_json::json;
    /// use std::time::Duration;
    /// use tokio::time::sleep;
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut socket = Socket::new(String::from("http://localhost:80"), None);
    ///
    ///     socket.on("foo", |message| println!("{}", message)).unwrap();
    ///     socket.connect().await.expect("Connection failed");
    ///
    ///     let payload = json!({"token": 123});
    ///     let ack = socket.emit_with_ack("foo", payload.to_string(), Duration::from_secs(2)).await.unwrap();
    ///
    ///     sleep(Duration::from_secs(2)).await;
    ///
    ///     if ack.read().expect("Server panicked anyway").acked {
    ///         println!("{}", ack.read().expect("Server panicked anyway").data.as_ref().unwrap());
    ///     }
    /// }
    /// ```
    pub async fn emit_with_ack<F>(
        &mut self,
        event: &str,
        data: String,
        timeout: Duration,
        callback: F,
    ) -> Result<(), Error>
    where
        F: Fn(String) + 'static + Send + Sync,
    {
        self.transport
            .emit_with_ack(event.into(), data, timeout, callback)
            .await
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use serde_json::json;

    #[actix_rt::test]
    async fn it_works() {
        let mut socket = Socket::new(String::from("http://localhost:4200"), None);

        let result = socket.on("test", |msg| println!("{}", msg));
        assert!(result.is_ok());

        let result = socket.connect().await;
        assert!(result.is_ok());

        let payload = json!({"token": 123});
        let result = socket.emit("test", payload.to_string()).await;

        assert!(result.is_ok());

        let ack_callback = |message: String| {
            println!("Yehaa! My ack got acked?");
            println!("Ack data: {}", message);
        };

        let ack = socket
            .emit_with_ack(
                "test",
                payload.to_string(),
                Duration::from_secs(2),
                ack_callback,
            )
            .await;
        assert!(ack.is_ok());

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
