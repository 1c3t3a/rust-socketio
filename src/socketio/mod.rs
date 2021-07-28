/// Defines the events that could be sent or received.
pub mod event;
mod packet;
/// Defines the types of payload (binary or string), that
/// could be sent or received.
pub mod payload;
pub mod socket;
pub(crate) mod transport;
