/// A type which represents a `payload` in the `socket.io` context.
/// A payload could either be of the type `Payload::Binary`, which holds
/// a [`std::vec::Vec<u8>`] that represents the payload or of the type
/// `Payload::String` which holds a [`std::string::String`]. The enum is
/// used for both representing data that's send and data that's received.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Payload {
    Binary(Vec<u8>),
    String(String),
}
