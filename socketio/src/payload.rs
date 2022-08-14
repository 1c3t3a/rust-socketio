use bytes::Bytes;

/// A type which represents a `payload` in the `socket.io` context.
/// A payload could either be of the type `Payload::Binary`, which holds
/// data in the [`Bytes`] type that represents the payload or of the type
/// `Payload::String` which holds a [`std::string::String`]. The enum is
/// used for both representing data that's send and data that's received.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Payload {
    Binary(Bytes),
    String(String),
}

impl From<&str> for Payload {
    fn from(string: &str) -> Self {
        Self::String(string.to_owned())
    }
}

impl From<String> for Payload {
    fn from(str: String) -> Self {
        Self::String(str)
    }
}

impl From<serde_json::Value> for Payload {
    fn from(value: serde_json::Value) -> Self {
        Self::String(value.to_string())
    }
}

impl From<Vec<u8>> for Payload {
    fn from(val: Vec<u8>) -> Self {
        Self::Binary(Bytes::from(val))
    }
}

impl From<&'static [u8]> for Payload {
    fn from(val: &'static [u8]) -> Self {
        Self::Binary(Bytes::from_static(val))
    }
}

impl From<Bytes> for Payload {
    fn from(bytes: Bytes) -> Self {
        Self::Binary(bytes)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_from() {
        let sut = Payload::from("foo ™");

        assert_eq!(Payload::String(String::from("foo ™")), sut);

        let sut = Payload::from(String::from("foo ™"));
        assert_eq!(Payload::String(String::from("foo ™")), sut);

        let sut = Payload::from(json!("foo ™"));
        assert_eq!(Payload::String(String::from("\"foo ™\"")), sut);

        let sut = Payload::from(vec![1, 2, 3]);
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(&[1_u8, 2_u8, 3_u8][..]);
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(Bytes::from_static(&[1, 2, 3]));
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);
    }
}
