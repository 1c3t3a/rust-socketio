[![Latest Version](https://img.shields.io/crates/v/rust_engineio)](https://crates.io/crates/rust_engine)
[![docs.rs](https://docs.rs/rust_engineio/badge.svg)](https://docs.rs/rust_engineio)

# Rust-engineio-client

An implementation of a engine.io client written in the rust programming language. This implementation currently supports revision 4 of the engine.io protocol. If you have any connection issues with this client, make sure the server uses at least revision 4 of the engine.io protocol.

## Example usage

``` rust
 use rust_engineio::{ClientBuilder, Client, packet::{Packet, PacketId}};
use url::Url;
use Bytes;

// get a client that is connected to the server
let client: Client = ClientBuilder::new(Url::parse("http://localhost:4201").unwrap())
     .on_open(|_| println!("Connection opened!"))
     .build()
     .expect("Connection failed");

// create a packet, in this case a message packet and emit it
let packet = Packet::new(PacketId::Message, &b"Hello World");
client.emit(packet).expect("Server unreachable");

// disconnect from the server
client.disconnect().expect("Disconnect failed")
```

The main entry point for using this crate is the `ClientBuilder` which provides
the opportunity to define how you want to connect to a certain endpoint. 
The following connection methods are available:
* `build`: Build websocket if allowed, if not fall back to polling. Standard configuration.
* `build_polling`: enforces a `polling` transport.
* `build_websocket_with_upgrade`: Build socket with a polling transport then upgrade to websocket transport (if possible).
* `build_websocket`: Build socket with only a websocket transport, crashes when websockets are not allowed.


## Current features

This implementation now supports all of the features of the engine.io protocol mentioned [here](https://github.com/socketio/engine.io-protocol).
This includes various transport options, the possibility of sending engine.io packets and registering the
commmon engine.io event callbacks:
* on_open
* on_close
* on_data
* on_error
* on_packet

It is also possible to pass in custom tls configurations via the `TlsConnector` as well
as custom headers for the opening request.

## Documentation

Documentation of this crate can be found up on [docs.rs](https://docs.rs/rust_engineio).
