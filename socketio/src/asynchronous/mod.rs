mod client;
mod generator;
mod socket;

#[cfg(all(feature = "async-callbacks", feature = "async"))]
pub use client::builder::ClientBuilder;
pub use client::client::Client;
