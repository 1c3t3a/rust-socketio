[![Latest Version](https://img.shields.io/crates/v/rust_socketio)](https://crates.io/crates/rust_socketio)

![tests](https://github.com/1c3t3a/rust-socketio/workflows/Rust/badge.svg)

# Rust-socketio-client

An implementation of a socket.io client written in the Rust programming language. This implementation currently supports revision 5 of the socket.io protocol and therefore revision 4 of the engine.io protocol. If you have any connection issues with this client, make sure the server uses at least revision 4 of the engine.io protocol.

## Example usage

``` rust
use rust_socketio::Socket;
use serde_json::json;

fn main() {
    // connect to a server on localhost with the /admin namespace and
    // a foo event handler
    let mut socket = SocketBuilder::new("http://localhost:80")
          .set_namespace("/admin")
          .expect("illegal namespace")
          .on("test", |str| println!("Received: {}", str))
          .connect()
          .expect("Connection failed");

    // emit to the "foo" event
    let payload = json!({"token": 123});
    socket.emit("foo", &payload.to_string()).expect("Server unreachable");

    // define a callback, that's executed when the ack got acked
    let ack_callback = |message: String| {
            println!("Yehaa! My ack got acked?");
            println!("Ack data: {}", message);
    };
    
    // emit with an ack
    let ack = socket
            .emit_with_ack("test", &payload.to_string(), Duration::from_secs(2), ack_callback)
            .expect("Server unreachable");
 }
```

The main entry point for using this crate is the `SocketBuilder` which provides a way to easily configure a socket in the needed way. When the `connect` method is called on the builder, it returns a connected client which then could be used to emit messages to certain events. One client can only be connected to one namespace. If you need to listen to the messages in different namespaces you need to allocate multiple sockets.

## Documentation

Documentation of this crate can be found up on [docs.rs](https://docs.rs/rust_socketio/0.1.0/rust_socketio/).

## Current features

This implementation support most of the features of the socket.io protocol. In general the full engine-io protocol is implemented, and concerning the socket.io part only binary events and binary acks are not yet implemented. This implementation generally tries to make use of websockets as often as possible. This means most times only the opening request uses http and as soon as the server mentions that he is able to use websockets, an upgrade is performed. But if this upgrade is not successful or the server does not mention an upgrade possibilty, http-long polling is used (as specified in the protocol specs).

Here's an overview of possible use-cases:

* connecting to a server.
* register callbacks for the following event types:
    - open
    - close
    - error
    - message
    - custom events like "foo", "on_payment", etc.
* send json-data to the server (recommended to use serde_json as it provides safe handling of json data).
* send json-data to the server and receive an ack with a possible message.

## Content of this repository

This repository contains a rust implementation of the socket.io protocol as well as the underlying engine.io protocol.

The details about the engine.io protocol can be found here:

* <https://github.com/socketio/engine.io-protocol>

The specification for the socket.io protocol here:

* <https://github.com/socketio/socket.io-protocol>

Looking at the component chart, the following parts are implemented (Source: https://socket.io/images/dependencies.jpg):

<img src="docs/res/dependencies.jpg" width="50%"/>

## Licence

MIT
