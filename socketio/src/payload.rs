use bytes::Bytes;

/// A type which represents a `payload` in the `socket.io` context.
/// A payload could either be of the type `Payload::Binary`, which holds
/// data in the [`Bytes`] type that represents the payload or of the type
/// `Payload::String` which holds a [`std::string::String`]. The enum is
/// used for both representing data that's send and data that's received.
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Payload {
    Binary(Bytes),
    Text(Vec<serde_json::Value>),
    #[deprecated = "Use `Payload::Text` instead. Continue existing behavior with: Payload::from(String)"]
    /// String that is sent as JSON if this is a JSON string, or as a raw string if it isn't
    String(String),
}

impl Payload {
    pub(crate) fn string_to_value(string: String) -> serde_json::Value {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&string) {
            value
        } else {
            serde_json::Value::String(string)
        }
    }
}

impl From<&str> for Payload {
    fn from(string: &str) -> Self {
        Payload::from(string.to_owned())
    }
}

impl From<String> for Payload {
    fn from(string: String) -> Self {
        Self::Text(vec![Payload::string_to_value(string)])
    }
}

impl From<Vec<String>> for Payload {
    fn from(arr: Vec<String>) -> Self {
        Self::Text(arr.into_iter().map(Payload::string_to_value).collect())
    }
}

impl From<Vec<serde_json::Value>> for Payload {
    fn from(values: Vec<serde_json::Value>) -> Self {
        Self::Text(values)
    }
}

impl From<serde_json::Value> for Payload {
    fn from(value: serde_json::Value) -> Self {
        Self::Text(vec![value])
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
    fn test_from_string() {
        let sut = Payload::from("foo ™");

        assert_eq!(
            Payload::Text(vec![serde_json::Value::String(String::from("foo ™"))]),
            sut
        );

        let sut = Payload::from(String::from("foo ™"));
        assert_eq!(
            Payload::Text(vec![serde_json::Value::String(String::from("foo ™"))]),
            sut
        );

        let sut = Payload::from(json!("foo ™"));
        assert_eq!(
            Payload::Text(vec![serde_json::Value::String(String::from("foo ™"))]),
            sut
        );
    }

    #[test]
    fn test_from_multiple_strings() {
        let input = vec![
            "one".to_owned(),
            "two".to_owned(),
            json!(["foo", "bar"]).to_string(),
        ];

        assert_eq!(
            Payload::Text(vec![
                serde_json::Value::String(String::from("one")),
                serde_json::Value::String(String::from("two")),
                json!(["foo", "bar"])
            ]),
            Payload::from(input)
        );
    }

    #[test]
    fn test_from_multiple_json() {
        let input = vec![json!({"foo": "bar"}), json!("foo"), json!(["foo", "bar"])];

        assert_eq!(Payload::Text(input.clone()), Payload::from(input.clone()));
    }

    #[test]
    fn test_from_json() {
        let json = json!({
            "foo": "bar"
        });
        let sut = Payload::from(json.clone());

        assert_eq!(Payload::Text(vec![json.clone()]), sut);

        // From JSON encoded string
        let sut = Payload::from(json.to_string());

        assert_eq!(Payload::Text(vec![json]), sut);
    }

    #[test]
    fn test_from_binary() {
        let sut = Payload::from(vec![1, 2, 3]);
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(&[1_u8, 2_u8, 3_u8][..]);
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);

        let sut = Payload::from(Bytes::from_static(&[1, 2, 3]));
        assert_eq!(Payload::Binary(Bytes::from_static(&[1, 2, 3])), sut);
    }
}
