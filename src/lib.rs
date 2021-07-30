//! Rust-socket.io is a socket.io client written in the Rust Programming Language.
//! ## Example usage
//!
//! ``` rust
//! use rust_socketio::{SocketBuilder, Payload, Socket};
//! use serde_json::json;
//! use std::time::Duration;
//!
//! // define a callback which is called when a payload is received
//! // this callback gets the payload as well as an instance of the
//! // socket to communicate with the server
//! let callback = |payload: Payload, mut socket: Socket| {
//!        match payload {
//!            Payload::String(str) => println!("Received: {}", str),
//!            Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
//!        }
//!        socket.emit("test", json!({"got ack": true})).expect("Server unreachable")
//! };
//!
//! // get a socket that is connected to the admin namespace
//! let mut socket = SocketBuilder::new("http://localhost:4200")
//!      .set_namespace("/admin")
//!      .expect("illegal namespace")
//!      .on("test", callback)
//!      .on("error", |err, _| eprintln!("Error: {:#?}", err))
//!      .connect()
//!      .expect("Connection failed");
//!
//! // emit to the "foo" event
//! let json_payload = json!({"token": 123});
//!
//! socket.emit("foo", json_payload).expect("Server unreachable");
//!
//! // define a callback, that's executed when the ack got acked
//! let ack_callback = |message: Payload, _| {
//!     println!("Yehaa! My ack got acked?");
//!     println!("Ack data: {:#?}", message);
//! };
//!
//! let json_payload = json!({"myAckData": 123});
//!
//! // emit with an ack
//! let ack = socket
//!     .emit_with_ack("test", json_payload, Duration::from_secs(2), ack_callback)
//!     .expect("Server unreachable");
//! ```
//!
//! The main entry point for using this crate is the [`SocketBuilder`] which provides
//! a way to easily configure a socket in the needed way. When the `connect` method
//! is called on the builder, it returns a connected client which then could be used
//! to emit messages to certain events. One client can only be connected to one namespace.
//! If you need to listen to the messages in different namespaces you need to
//! allocate multiple sockets.
//!
//! ## Current features
//!
//! This implementation now supports all of the features of the socket.io protocol mentioned
//! [here](https://github.com/socketio/socket.io-protocol).
//! It generally tries to make use of websockets as often as possible. This means most times
//! only the opening request uses http and as soon as the server mentions that he is able to use
//! websockets, an upgrade  is performed. But if this upgrade is not successful or the server
//! does not mention an upgrade possibility, http-long polling is used (as specified in the protocol specs).
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
//! - send and handle Binary data.
//!
#![allow(clippy::rc_buffer)]
#![warn(clippy::complexity)]
#![warn(clippy::style)]
#![warn(clippy::perf)]
#![warn(clippy::correctness)]
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

pub use reqwest::header::{HeaderMap, HeaderValue, IntoHeaderName};
pub use socketio::{event::Event, payload::Payload};

pub use socketio::socket::Socket;
pub use socketio::socket::SocketBuilder;

#[cfg(test)]
mod test {

    use std::thread::sleep;

    use super::*;
    use bytes::Bytes;
    use native_tls::TlsConnector;
    use reqwest::header::{ACCEPT_ENCODING, HOST};
    use serde_json::json;
    use socketio::payload::Payload;
    use std::time::Duration;

    const SERVER_URL: &str = "http://localhost:4200";

    #[test]
    fn it_works() {
        let url = std::env::var("SOCKET_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

        let mut socket = Socket::new(url, None, None, None);

        let result = socket.on(
            "test".into(),
            Box::new(|msg, _| match msg {
                Payload::String(str) => println!("Received string: {}", str),
                Payload::Binary(bin) => println!("Received binary data: {:#?}", bin),
            }),
        );
        assert!(result.is_ok());

        let result = socket.connect();
        assert!(result.is_ok());

        let payload = json!({"token": 123});
        let result = socket.emit("test", Payload::String(payload.to_string()));

        assert!(result.is_ok());

        let ack_callback = move |message: Payload, mut socket_: Socket| {
            let result = socket_.emit(
                "test",
                Payload::String(json!({"got ack": true}).to_string()),
            );
            assert!(result.is_ok());

            println!("Yehaa! My ack got acked?");
            if let Payload::String(str) = message {
                println!("Received string Ack");
                println!("Ack data: {}", str);
            }
        };

        let ack = socket.emit_with_ack(
            "test",
            Payload::String(payload.to_string()),
            Duration::from_secs(2),
            ack_callback,
        );
        assert!(ack.is_ok());

        socket.disconnect().unwrap();
        // assert!(socket.disconnect().is_ok());

        sleep(Duration::from_secs(10));
    }

    #[test]
    fn test_builder() {
        let url = std::env::var("SOCKET_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());

        // expect an illegal namespace
        assert!(SocketBuilder::new(url.clone())
            .set_namespace("illegal")
            .is_err());

        // test socket build logic
        let socket_builder = SocketBuilder::new(url);

        let tls_connector = TlsConnector::builder()
            .use_sni(true)
            .build()
            .expect("Found illegal configuration");

        let socket = socket_builder
            .set_namespace("/")
            .expect("Error!")
            .set_tls_config(tls_connector)
            .set_opening_header(HOST, "localhost".parse().unwrap())
            .set_opening_header(ACCEPT_ENCODING, "application/json".parse().unwrap())
            .on("test", |str, _| println!("Received: {:#?}", str))
            .on("message", |payload, _| println!("{:#?}", payload))
            .connect();

        assert!(socket.is_ok());

        let mut socket = socket.unwrap();
        assert!(socket.emit("message", json!("Hello World")).is_ok());

        assert!(socket.emit("binary", Bytes::from_static(&[46, 88])).is_ok());

        let ack_cb = |payload, _| {
            println!("Yehaa the ack got acked");
            println!("With data: {:#?}", payload);
        };

        assert!(socket
            .emit_with_ack("binary", json!("pls ack"), Duration::from_secs(1), ack_cb,)
            .is_ok());

        sleep(Duration::from_secs(2));
    }
}
