[![Latest Version](https://img.shields.io/crates/v/rust_socketio)](https://crates.io/crates/rust_socketio)
![tests](https://github.com/1c3t3a/rust-socketio/workflows/Rust/badge.svg)

# Rust-socketio

An asynchronous implementation of a socket.io client written in the Rust programming language.

## Example usage

``` rust
use rust_socketio::Socket;
use serde_json::json;
use tokio::time::sleep;

#[tokio::main]
async fn main() {
    let mut socket = Socket::new(String::from("http://localhost:80"), Some("/admin"));

    // callback for the "foo" event
    socket.on("foo", |message| println!("{}", message)).unwrap();

    // connect to the server
    socket.connect().await.expect("Connection failed");

    // emit to the "foo" event
    let payload = json!({"token": 123});
    socket.emit("foo", payload.to_string()).await.expect("Server unreachable");

    // emit with an ack
    let ack = socket.emit_with_ack("foo", payload.to_string(), Duration::from_secs(2)).await.unwrap();

    sleep(Duration::from_secs(2)).await;

    // check if ack is present and read the data
    if ack.read().expect("Server panicked anyway").acked {
        println!("{}", ack.read().expect("Server panicked anyway").data.as_ref().unwrap());
    }
 }
```

## Documentation

Documentation of this crate can be found up on [docs.rs](https://docs.rs/rust_socketio/0.1.0/rust_socketio/).


## Current features

This is the first released version of the client, so it still lacks some features that the normal client would provide. First of all the underlying engine.io protocol still uses long-polling instead of websockets. This will be resolved as soon as both the reqwest libary as well as tungstenite-websockets will bump their tokio version to 1.0.0. At the moment only reqwest is used for async long polling. In general the full engine-io protocol is implemented and most of the features concerning the 'normal' socket.io protocol work as well.

Here's an overview of possible use-cases:

- connecting to a server.
- register callbacks for the following event types:
    - open
    - close
    - error
    - message
    - custom events like "foo", "on_payment", etc.
- send json-data to the server (recommended to use serde_json as it provides safe handling of json data).
- send json-data to the server and receive an ack with a possible message.

What's currently missing is the emitting of binary data - I aim to implement this as soon as possible.

The whole crate is written in asynchronous rust and it's necessary to use [tokio](https://docs.rs/tokio/1.0.1/tokio/), or other executors with this libary to resolve the futures.

## Content of this repository

This repository contains a rust implementation of the socket.io protocol as well as the underlying engine.io protocol.

The details about the engine.io protocol can be found here:

- <https://github.com/socketio/engine.io-protocol>

The specification for the socket.io protocol here:

- <https://github.com/socketio/socket.io-protocol>

Looking at the component chart, the following parts are implemented (Source: https://socket.io/images/dependencies.jpg):

<img src="docs/res/dependencies.jpg" width="50%"/>

## Licence

MIT
