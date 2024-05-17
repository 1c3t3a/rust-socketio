mod builder;
mod raw_client;

pub use builder::{ClientBuilder, EngineIOPacket, PacketSerializer, TransportType};
pub use client::Client;
pub use raw_client::RawClient;

/// Internal callback type
mod callback;
mod client;
