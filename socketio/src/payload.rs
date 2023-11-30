use bytes::Bytes;

/// A type which represents a `payload` in the `socket.io` context.
/// A payload could either be of the type `Payload::Binary`, which holds
/// data in the [`Bytes`] type that represents the payload, of the type
/// `Payload::String` which holds a [`std::string::String`] or of the type
/// `Payload::StringArray` which holds a [`Vec<std::string::String>`]. The enum is
/// used for both representing data that's send and data that's received.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Payload {
    Binary(Bytes),
    String(String),
    StringArray(Vec<String>),
}

impl From<&str> for Payload {
    fn from(string: &str) -> Self {
        Self::String(string.to_owned())
    }
}

impl From<Vec<&str>> for Payload {
    fn from(strings: Vec<&str>) -> Self {
        Self::StringArray(strings.iter().map(|str| str.to_string()).collect())
    }
}

impl From<String> for Payload {
    fn from(str: String) -> Self {
        Self::String(str)
    }
}

impl From<Vec<String>> for Payload {
    fn from(arr: Vec<String>) -> Self {
        Self::StringArray(arr)
    }
}

impl From<serde_json::Value> for Payload {
    fn from(value: serde_json::Value) -> Self {
        Self::String(value.to_string())
    }
}

impl From<Vec<serde_json::Value>> for Payload {
    fn from(values: Vec<serde_json::Value>) -> Self {
        Self::StringArray(values.iter().map(|value| value.to_string()).collect())
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

impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        match self {
            Payload::Binary(b) => b.as_ref(),
            Payload::String(s) => s.as_ref(),
            // TODO: how to handle this case
            Payload::StringArray(_a) => &[0u8; 0],
        }
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

        let sut = Payload::from(vec!["foo ™", "bar ™"]);
        assert_eq!(
            Payload::StringArray(vec!["foo ™".to_owned(), "bar ™".to_owned()]),
            sut
        );

        let sut = Payload::from(vec!["foo ™".to_owned(), "bar ™".to_owned()]);
        assert_eq!(
            Payload::StringArray(vec!["foo ™".to_owned(), "bar ™".to_owned()]),
            sut
        );

        let sut = Payload::from(vec![json!("foo ™"), json!("bar ™")]);
        assert_eq!(
            Payload::StringArray(vec!["\"foo ™\"".to_owned(), "\"bar ™\"".to_owned()]),
            sut
        );
    }
}
