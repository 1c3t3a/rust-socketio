use bytes::Bytes;
use serde_json::Value;

/// A type which represents a `payload` in the `socket.io` context.
/// The enum is used for both representing data that's send and
/// data that's received.
#[derive(Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub enum Payload {
    Binary(Bytes),
    Json(Value),
    Multi(Vec<RawPayload>),
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RawPayload {
    Binary(Bytes),
    Json(Value),
}

impl From<serde_json::Value> for Payload {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl From<Option<serde_json::Value>> for Payload {
    fn from(value: Option<serde_json::Value>) -> Self {
        match value {
            None => Self::Json(Value::Null),
            Some(value) => Self::Json(value),
        }
    }
}

impl From<Vec<serde_json::Value>> for Payload {
    fn from(value: Vec<serde_json::Value>) -> Self {
        if value.len() == 1 {
            Self::Json(value.first().unwrap().to_owned())
        } else {
            Self::Multi(value.into_iter().map(|v| v.into()).collect())
        }
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

impl From<serde_json::Value> for RawPayload {
    fn from(value: serde_json::Value) -> Self {
        Self::Json(value)
    }
}

impl From<Vec<u8>> for RawPayload {
    fn from(val: Vec<u8>) -> Self {
        Self::Binary(Bytes::from(val))
    }
}

impl From<&'static [u8]> for RawPayload {
    fn from(val: &'static [u8]) -> Self {
        Self::Binary(Bytes::from_static(val))
    }
}

impl From<Bytes> for RawPayload {
    fn from(bytes: Bytes) -> Self {
        Self::Binary(bytes)
    }
}

impl From<RawPayload> for Payload {
    fn from(val: RawPayload) -> Self {
        match val {
            RawPayload::Json(data) => Payload::Json(data),
            RawPayload::Binary(bin) => Payload::Binary(bin),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_from() {
        let sut = Payload::from(json!("foo ™"));
        assert_eq!(Payload::Json(json!("foo ™")), sut);

        let sut = Payload::from(vec![1, 2, 3]);
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(&[1_u8, 2_u8, 3_u8][..]);
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(Bytes::from_static(&[1, 2, 3]));
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(json!(5));
        assert_eq!(Payload::Json(json!(5)), sut);

        let sut = Payload::from(json!("5"));
        assert_eq!(Payload::Json(json!("5")), sut);
    }
}
