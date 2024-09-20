//! # Rust-engineio-client
//!
//! An implementation of a engine.io client written in the rust programming language. This implementation currently
//! supports revision 4 of the engine.io protocol. If you have any connection issues with this client,
//! make sure the server uses at least revision 4 of the engine.io protocol.
//!
//! ## Example usage
//!
//! ``` rust
//! use rust_engineio::{ClientBuilder, Client, packet::{Packet, PacketId}};
//! use url::Url;
//! use bytes::Bytes;
//!
//! // get a client with an `on_open` callback
//! let client: Client = ClientBuilder::new(Url::parse("http://localhost:4201").unwrap())
//!      .on_open(|_| println!("Connection opened!"))
//!      .build()
//!      .expect("Creating client failed");
//!
//! // connect to the server
//! client.connect().expect("Connection failed");
//!
//! // create a packet, in this case a message packet and emit it
//! let packet = Packet::new(PacketId::Message, Bytes::from_static(b"Hello World"));
//! client.emit(packet).expect("Server unreachable");
//!
//! // disconnect from the server
//! client.disconnect().expect("Disconnect failed")
//! ```
//!
//! The main entry point for using this crate is the [`ClientBuilder`] (or [`asynchronous::ClientBuilder`] respectively)
//! which provides the opportunity to define how you want to connect to a certain endpoint.
//! The following connection methods are available:
//! * `build`: Build websocket if allowed, if not fall back to polling. Standard configuration.
//! * `build_polling`: enforces a `polling` transport.
//! * `build_websocket_with_upgrade`: Build socket with a polling transport then upgrade to websocket transport (if possible).
//! * `build_websocket`: Build socket with only a websocket transport, crashes when websockets are not allowed.
//!
//!
//! ## Current features
//!
//! This implementation now supports all of the features of the engine.io protocol mentioned [here](https://github.com/socketio/engine.io-protocol).
//! This includes various transport options, the possibility of sending engine.io packets and registering the
//! common engine.io event callbacks:
//! * on_open
//! * on_close
//! * on_data
//! * on_error
//! * on_packet
//!
//! It is also possible to pass in custom tls configurations via the `TlsConnector` as well
//! as custom headers for the opening request.
//!
//! ## Async version
//!
//! The crate also ships with an asynchronous version that can be enabled with a feature flag.
//! The async version implements the same features mentioned above.
//! The asynchronous version has a similar API, just with async functions. Currently the futures
//! can only be executed with [`tokio`](https://tokio.rs). In the first benchmarks the async version
//! showed improvements of up to 93% in speed.
//! To make use of the async version, import the crate as follows:
//! ```toml
//! [depencencies]
//! rust-engineio = { version = "0.3.1", features = ["async"] }
//! ```
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
        std::thread::scope(|s| {
            s.spawn(|| $e);
        });
    };
}

pub mod asynchronous;
mod callback;
pub mod client;
/// Generic header map
pub mod header;
pub mod packet;
pub(self) mod socket;
pub mod transport;
pub mod transports;

pub const ENGINE_IO_VERSION: i32 = 4;

/// Contains the error type which will be returned with every result in this
/// crate. Handles all kinds of errors.
pub mod error;

pub use client::{Client, ClientBuilder};
pub use error::Error;
pub use packet::{Packet, PacketId};

// Re-export TLS configurations to make sockio integration easier
#[cfg(all(feature = "_native-tls", not(feature = "_rustls-tls")))]
#[doc(hidden)]
pub use native_tls::TlsConnector as TlsConfig;
#[doc(hidden)]
#[cfg(feature = "_rustls-tls")]
pub use rustls::ClientConfig as TlsConfig;

// Both native-tls and rustls is not supported at the same time
#[cfg(not(feature = "_fallback-tls"))]
#[cfg(all(feature = "_native-tls", feature = "_rustls-tls"))]
compile_error!("Both native-tls and rustls features are enabled. Please enable only one of them.");

#[cfg(not(any(feature = "_native-tls", feature = "_rustls-tls")))]
compile_error!("No TLS feature is enabled. Please enable either native-tls or rustls.");

#[cfg(test)]
pub(crate) mod test {
    use super::*;
    use native_tls::TlsConnector;
    const CERT_PATH: &str = "../ci/cert/ca.crt";
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
    /// The `engine.io` server for testing runs on port 4201
    const SERVER_URL: &str = "http://localhost:4201";
    /// The `engine.io` server that refuses upgrades runs on port 4203
    const SERVER_POLLING_URL: &str = "http://localhost:4203";
    const SERVER_URL_SECURE: &str = "https://localhost:4202";
    use url::Url;

    pub(crate) fn engine_io_server() -> crate::error::Result<Url> {
        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());
        Ok(Url::parse(&url)?)
    }

    pub(crate) fn engine_io_polling_server() -> crate::error::Result<Url> {
        let url = std::env::var("ENGINE_IO_POLLING_SERVER")
            .unwrap_or_else(|_| SERVER_POLLING_URL.to_owned());
        Ok(Url::parse(&url)?)
    }

    pub(crate) fn engine_io_server_secure() -> crate::error::Result<Url> {
        let url = std::env::var("ENGINE_IO_SECURE_SERVER")
            .unwrap_or_else(|_| SERVER_URL_SECURE.to_owned());
        Ok(Url::parse(&url)?)
    }
}
