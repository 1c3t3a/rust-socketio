#[macro_use]
mod engineio;
pub mod socketio;
pub mod util;

use crate::util::Error;
use std::{
    sync::{Arc, RwLock},
    time::Duration,
};

use crate::socketio::{ack::Ack, transport::TransportClient};

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
            transport: TransportClient::new(address, namespace.map(|s| String::from(s))),
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
    /// or a custom event like "foo", as well as a String. But be careful, the string needs to be valid json.
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
    /// or a custom event like "foo", as well as a String. But be careful, the string needs to be valid json.
    /// It's even recommended to use a libary like serde_json to serialize your data properly.
    /// It also requires a timeout, a `Duration` in which the client needs to answer.
    /// This method returns an `Arc<RwLock<Ack>>`. The `Ack` type holds information about the Ack, which are whether
    /// the ack got acked fast enough and potential data. It is safe to unwrap the data after the ack got acked from the server.
    /// This uses an RwLock to reach shared mutability, which is needed as the server set's the data on the ack later.
    /// # Example
    /// ```
    /// use rust_socketio::Socket;
    /// use serde_json::{Value, json};
    /// use std::time::Duration;
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
    ///     tokio::time::delay_for(Duration::from_secs(2)).await;
    ///
    ///     if ack.read().expect("Server panicked anyway").acked {
    ///         println!("{}", ack.read().expect("Server panicked anyway").data.as_ref().unwrap());
    ///     }
    /// }
    /// ```
    pub async fn emit_with_ack(
        &mut self,
        event: &str,
        data: String,
        timeout: Duration,
    ) -> Result<Arc<RwLock<Ack>>, Error> {
        self.transport
            .emit_with_ack(event.into(), data, timeout)
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

        let ack = socket.emit_with_ack("test", payload.to_string(), Duration::from_secs(2)).await;
        assert!(ack.is_ok());
        let ack = ack.unwrap();

        tokio::time::delay_for(Duration::from_secs(2)).await;

        println!("{}", ack.clone().read().unwrap().acked);
        if let Some(data) = ack.clone().read().unwrap().data.as_ref() {
            println!("{}", data);
        }
    }
}
