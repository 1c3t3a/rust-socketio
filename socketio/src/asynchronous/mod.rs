mod client;
mod generator;
mod socket;

#[cfg(feature = "async")]
pub use client::builder::ClientBuilder;
pub use client::client::{Client, ReconnectSettings};

#[doc = r#"
A macro to wrap an async callback function to be used in the client.

This macro is used to wrap a callback function that can handle a specific event.

```rust
pub async fn callback(payload: Payload, client: Client) {}

let socket = ClientBuilder::new(host)
        .on("message", async_callback!(callback))
        .connect()
        .await
        .expect("Connection failed");
```
"#]
#[macro_export]
macro_rules! async_callback {
    ($f:expr) => {
        |payload: Payload, client: Client| $f(payload, client).boxed()
    };
}

#[doc = r#"
A macro to wrap an async callback function to be used in the client.

This macro is used to wrap a callback function that can handle any event.

```rust
pub async fn callback_any(event: Event, payload: Payload, client: Client) {}

let socket = ClientBuilder::new(host)
        .on_any(async_any_callback!(callback_any))
        .connect()
        .await
        .expect("Connection failed");
```
"#]
#[macro_export]
macro_rules! async_any_callback {
    ($f:expr) => {
        |event: Event, payload: Payload, client: Client| $f(event, payload, client).boxed()
    };
}
