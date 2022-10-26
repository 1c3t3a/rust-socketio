//! Rust-socket.io is a socket.io client written in the Rust Programming Language.
//! ## Example usage
//!
//! ``` rust
//! use rust_socketio::{ClientBuilder, Payload, RawClient};
//! use serde_json::json;
//! use std::time::Duration;
//!
//! // define a callback which is called when a payload is received
//! // this callback gets the payload as well as an instance of the
//! // socket to communicate with the server
//! let callback = |payload: Payload, socket: RawClient| {
//!        match payload {
//!            Payload::Json(data) => println!("Received: {}", data),
//!            Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
//!            Payload::Multi(vec) => println!("Received multi: {:?}", vec),
//!            _ => {}
//!        }
//!        socket.emit("test", json!({"got ack": true})).expect("Server unreachable")
//! };
//!
//! // get a socket that is connected to the admin namespace
//! let mut socket = ClientBuilder::new("http://localhost:4200/")
//!      .namespace("/admin")
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
//! let ack_callback = |message: Payload, _: RawClient| {
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
//! The main entry point for using this crate is the [`ClientBuilder`] which provides
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

/// Defines client only structs
pub mod client;
/// Deprecated import since 0.3.0-alpha-2, use Event in the crate root instead.
/// Defines the events that could be sent or received.
pub mod event;
pub(crate) mod packet;
/// Deprecated import since 0.3.0-alpha-2, use Event in the crate root instead.
/// Defines the types of payload (binary or string), that
/// could be sent or received.
pub mod payload;
pub(self) mod socket;

/// Deprecated import since 0.3.0-alpha-2, use Error in the crate root instead.
/// Contains the error type which will be returned with every result in this
/// crate.
pub mod error;

pub use error::Error;

pub use {event::Event, payload::Payload};

pub use client::{ClientBuilder, RawClient, TransportType};

// TODO: 0.4.0 remove
#[deprecated(since = "0.3.0-alpha-2", note = "Socket renamed to Client")]
pub use client::{ClientBuilder as SocketBuilder, RawClient as Socket};

#[cfg(test)]
pub(crate) mod test {
    use url::Url;

    /// The socket.io server for testing runs on port 4200
    const SERVER_URL: &str = "http://localhost:4200";

    pub(crate) fn socket_io_server() -> Url {
        let url = std::env::var("SOCKET_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());
        let mut url = Url::parse(&url).unwrap();

        if url.path() == "/" {
            url.set_path("/socket.io/");
        }

        url
    }

    // The socket.io auth server for testing runs on port 4204
    const AUTH_SERVER_URL: &str = "http://localhost:4204";

    pub(crate) fn socket_io_auth_server() -> Url {
        let url =
            std::env::var("SOCKET_IO_AUTH_SERVER").unwrap_or_else(|_| AUTH_SERVER_URL.to_owned());
        let mut url = Url::parse(&url).unwrap();

        if url.path() == "/" {
            url.set_path("/socket.io/");
        }

        url
    }

    // The socket.io restart server for testing runs on port 4205
    const RESTART_SERVER_URL: &str = "http://localhost:4205";

    pub(crate) fn socket_io_restart_server() -> Url {
        let url = std::env::var("SOCKET_IO_RESTART_SERVER")
            .unwrap_or_else(|_| RESTART_SERVER_URL.to_owned());
        let mut url = Url::parse(&url).unwrap();

        if url.path() == "/" {
            url.set_path("/socket.io/");
        }

        url
    }
}
