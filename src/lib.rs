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
//! let mut socket = SocketBuilder::new("http://localhost:4200/")
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
pub(crate) mod test {
    use super::*;
    use native_tls::TlsConnector;
    const CERT_PATH: &str = "ci/cert/ca.crt";
    use native_tls::Certificate;
    use std::fs::File;
    use std::io::Read;

    pub(crate) fn tls_connector() -> error::Result<TlsConnector> {
        let cert_path = std::env::var("CA_CERT_PATH").unwrap_or_else(|_| CERT_PATH.to_owned());
        let mut cert_file = File::open(cert_path)?;
        let mut buf = vec![];
        cert_file.read_to_end(&mut buf)?;
        let cert: Certificate = Certificate::from_pem(&buf[..]).unwrap();
        Ok(TlsConnector::builder()
            // ONLY USE FOR TESTING!
            .danger_accept_invalid_hostnames(true)
            .add_root_certificate(cert)
            .build()
            .unwrap())
    }
}
