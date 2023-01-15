[![Latest Version](https://img.shields.io/crates/v/rust_socketio)](https://crates.io/crates/rust_socketio)
[![docs.rs](https://docs.rs/rust_socketio/badge.svg)](https://docs.rs/rust_socketio)
[![Build and code style](https://github.com/1c3t3a/rust-socketio/actions/workflows/build.yml/badge.svg)](https://github.com/1c3t3a/rust-socketio/actions/workflows/build.yml)
[![Test](https://github.com/1c3t3a/rust-socketio/actions/workflows/test.yml/badge.svg)](https://github.com/1c3t3a/rust-socketio/actions/workflows/test.yml)
[![codecov](https://codecov.io/gh/1c3t3a/rust-socketio/branch/main/graph/badge.svg?token=GUF406K0KL)](https://codecov.io/gh/1c3t3a/rust-socketio)

# Rust-socketio-client

An implementation of a socket.io client written in the rust programming language. This implementation currently supports revision 5 of the socket.io protocol and therefore revision 4 of the engine.io protocol. If you have any connection issues with this client, make sure the server uses at least revision 4 of the engine.io protocol.
Information on the [`async`](#async) version can be found below.

## Example usage

Add the following to your `Cargo.toml` file:

```toml
rust_socketio = "0.4.1"
```

Then you're able to run the following example code:

``` rust
use rust_socketio::{ClientBuilder, Payload, RawClient};
use serde_json::json;
use std::time::Duration;

// define a callback which is called when a payload is received
// this callback gets the payload as well as an instance of the
// socket to communicate with the server
let callback = |payload: Payload, socket: RawClient| {
       match payload {
           Payload::String(str) => println!("Received: {}", str),
           Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
       }
       socket.emit("test", json!({"got ack": true})).expect("Server unreachable")
};

// get a socket that is connected to the admin namespace
let socket = ClientBuilder::new("http://localhost:4200")
     .namespace("/admin")
     .on("test", callback)
     .on("error", |err, _| eprintln!("Error: {:#?}", err))
     .connect()
     .expect("Connection failed");

// emit to the "foo" event
let json_payload = json!({"token": 123});
socket.emit("foo", json_payload).expect("Server unreachable");

// define a callback, that's executed when the ack got acked
let ack_callback = |message: Payload, _| {
    println!("Yehaa! My ack got acked?");
    println!("Ack data: {:#?}", message);
};

let json_payload = json!({"myAckData": 123});
// emit with an ack
socket
    .emit_with_ack("test", json_payload, Duration::from_secs(2), ack_callback)
    .expect("Server unreachable");

socket.disconnect().expect("Disconnect failed")

```

The main entry point for using this crate is the `ClientBuilder` which provides a way to easily configure a socket in the needed way. When the `connect` method is called on the builder, it returns a connected client which then could be used to emit messages to certain events. One client can only be connected to one namespace. If you need to listen to the messages in different namespaces you need to allocate multiple sockets.

## Documentation

Documentation of this crate can be found up on [docs.rs](https://docs.rs/rust_socketio).

## Current features

This implementation now supports all of the features of the socket.io protocol mentioned [here](https://github.com/socketio/socket.io-protocol).
It generally tries to make use of websockets as often as possible. This means most times
only the opening request uses http and as soon as the server mentions that he is able to upgrade to
websockets, an upgrade  is performed. But if this upgrade is not successful or the server
does not mention an upgrade possibility, http-long polling is used (as specified in the protocol specs).
Here's an overview of possible use-cases:
- connecting to a server.
- register callbacks for the following event types:
    - open
    - close
    - error
    - message
    - custom events like "foo", "on_payment", etc.
- send JSON data to the server (via `serde_json` which provides safe
handling).
- send JSON data to the server and receive an `ack`.
- send and handle Binary data.

## <a name="async"> Async version
This library provides an ability for being executed in an asynchronous context using `tokio` as
the execution runtime.
Please note that the current async implementation is still experimental, the interface can be object to
changes at any time.
The async `Client` and `ClientBuilder` support a similar interface to the sync version and live
in the `asynchronous` module. In order to enable the support, you need to enable the `async`
feature flag:
```toml
rust_socketio = { version = "0.4.1-alpha.1", features = ["async"] }
```

The following code shows the example above in async fashion:
``` rust
use futures_util::FutureExt;
use rust_socketio::{
    asynchronous::{Client, ClientBuilder},
    Payload,
};
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() {
    // define a callback which is called when a payload is received
    // this callback gets the payload as well as an instance of the
    // socket to communicate with the server
    let callback = |payload: Payload, socket: Client| {
        async move {
            match payload {
                Payload::String(str) => println!("Received: {}", str),
                Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
            }
            socket
                .emit("test", json!({"got ack": true}))
                .await
                .expect("Server unreachable");
        }
        .boxed()
    };

    // get a socket that is connected to the admin namespace
    let socket = ClientBuilder::new("http://localhost:4200/")
        .namespace("/admin")
        .on("test", callback)
        .on("error", |err, _| {
            async move { eprintln!("Error: {:#?}", err) }.boxed()
        })
        .connect()
        .await
        .expect("Connection failed");

    // emit to the "foo" event
    let json_payload = json!({"token": 123});
    socket
        .emit("foo", json_payload)
        .await
        .expect("Server unreachable");

    // define a callback, that's executed when the ack got acked
    let ack_callback = |message: Payload, _: Client| {
        async move {
            println!("Yehaa! My ack got acked?");
            println!("Ack data: {:#?}", message);
        }
        .boxed()
    };

    let json_payload = json!({"myAckData": 123});
    // emit with an ack
    socket
        .emit_with_ack("test", json_payload, Duration::from_secs(2), ack_callback)
        .await
        .expect("Server unreachable");

    socket.disconnect().await.expect("Disconnect failed");
}
```

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
