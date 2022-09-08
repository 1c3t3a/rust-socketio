pub mod async_transports;
pub mod transport;

pub(self) mod async_socket;
#[cfg(feature = "async-callbacks")]
mod callback;
#[cfg(feature = "async")]
pub mod client;
mod generator;
#[cfg(feature = "async")]
pub mod server;

pub use client::{Client, ClientBuilder};
#[cfg(feature = "async")]
pub use server::Sid;
