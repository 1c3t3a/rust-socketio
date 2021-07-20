/// Defines the events that could be sent or received.
pub mod event;
mod packet;
/// Defines the types of payload (binary or string), that
/// could be sent or received.
pub mod payload;
pub(crate) mod socket;
pub mod socket_builder;

// For ease of access
pub type SocketBuilder = socket_builder::SocketBuilder;
pub type Socket = socket::SocketIOSocket;
