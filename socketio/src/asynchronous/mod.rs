mod ack;
mod client;
mod generator;
mod server;
mod socket;

#[cfg(feature = "async-callbacks")]
pub(crate) mod callback;
#[cfg(feature = "async")]
pub use client::builder::ClientBuilder;
pub use client::client::Client;
#[cfg(feature = "async")]
pub use server::builder::ServerBuilder;
pub use server::client::Client as ServerClient;

pub(crate) type NameSpace = String;
