[![Latest Version](https://img.shields.io/crates/v/rust_engineio)](https://crates.io/crates/rust_engineio)
[![docs.rs](https://docs.rs/rust_engineio/badge.svg)](https://docs.rs/rust_engineio)

# Rust-engineio-client

An implementation of a engine.io client written in the rust programming language. This implementation currently supports revision 4 of the engine.io protocol. If you have any connection issues with this client, make sure the server uses at least revision 4 of the engine.io protocol.

## Example usage

``` rust
use rust_engineio::{ClientBuilder, Client, packet::{Packet, PacketId}};
use url::Url;
use bytes::Bytes;

// get a client with an `on_open` callback
let client: Client = ClientBuilder::new(Url::parse("http://localhost:4201").unwrap())
     .on_open(|_| println!("Connection opened!"))
     .build()
     .expect("Connection failed");

// connect to the server
client.connect().expect("Connection failed");

// create a packet, in this case a message packet and emit it
let packet = Packet::new(PacketId::Message, Bytes::from_static(b"Hello World"));
client.emit(packet).expect("Server unreachable");

// disconnect from the server
client.disconnect().expect("Disconnect failed")
```

The main entry point for using this crate is the `ClientBuilder` (or `asynchronous::ClientBuilder` respectively)
which provides the opportunity to define how you want to connect to a certain endpoint. 
The following connection methods are available:
* `build`: Build websocket if allowed, if not fall back to polling. Standard configuration.
* `build_polling`: enforces a `polling` transport.
* `build_websocket_with_upgrade`: Build socket with a polling transport then upgrade to websocket transport (if possible).
* `build_websocket`: Build socket with only a websocket transport, crashes when websockets are not allowed.


## Current features

This implementation now supports all of the features of the engine.io protocol mentioned [here](https://github.com/socketio/engine.io-protocol).
This includes various transport options, the possibility of sending engine.io packets and registering the
common engine.io event callbacks:
* on_open
* on_close
* on_data
* on_error
* on_packet

It is also possible to pass in custom tls configurations via the `TlsConnector` as well
as custom headers for the opening request.

## Documentation

Documentation of this crate can be found up on [docs.rs](https://docs.rs/rust_engineio).

## Async version

The crate also ships with an asynchronous version that can be enabled with a feature flag.
The async version implements the same features mentioned above.
The asynchronous version has a similar API, just with async functions. Currently the futures 
can only be executed with [`tokio`](https://tokio.rs). The async version also introduces the
`asynchronous::context::Context` type, where an instance needs to be created once.
To make use of the async version, import the crate as follows:
```toml
[depencencies]
rust-engineio = { version = "0.3.1", features = ["async"] }
```
