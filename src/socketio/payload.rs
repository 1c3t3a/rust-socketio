#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Payload {
    Binary(Vec<u8>),
    String(String),
}
