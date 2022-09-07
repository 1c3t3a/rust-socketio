pub mod async_transports;
pub mod transport;

#[cfg(feature = "async")]
pub(self) mod async_socket;
#[cfg(feature = "async")]
mod callback;
#[cfg(feature = "async")]
pub mod client;
#[cfg(feature = "async")]
mod generator;
#[cfg(feature = "async")]
pub mod server;

#[cfg(feature = "async")]
pub use client::{Client, ClientBuilder};
#[cfg(feature = "async")]
pub use server::Sid;
