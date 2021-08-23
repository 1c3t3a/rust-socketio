use crate::Error;
use bytes::Bytes;
use reqwest::header::{
    HeaderMap as ReqwestHeaderMap, HeaderName as ReqwestHeaderName,
    HeaderValue as ReqwestHeaderValue,
};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::str::FromStr;
use websocket::header::Headers as WebSocketHeaderMap;

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub struct HeaderName {
    inner: String,
}

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub struct HeaderValue {
    inner: Bytes,
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct HeaderMap {
    map: HashMap<HeaderName, HeaderValue>,
}

pub struct IntoIter {
    inner: std::collections::hash_map::IntoIter<HeaderName, HeaderValue>,
}

impl ToString for HeaderName {
    fn to_string(&self) -> std::string::String {
        self.inner.clone()
    }
}

impl From<String> for HeaderName {
    fn from(string: String) -> Self {
        HeaderName { inner: string }
    }
}

impl FromStr for HeaderName {
    type Err = Error;
    fn from_str(value: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        Ok(HeaderName::from(value.to_owned()))
    }
}

impl TryFrom<HeaderName> for ReqwestHeaderName {
    type Error = Error;
    fn try_from(
        header: HeaderName,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderName>>::Error> {
        Ok(ReqwestHeaderName::from_str(&header.to_string())?)
    }
}

impl From<ReqwestHeaderName> for HeaderName {
    fn from(header: ReqwestHeaderName) -> Self {
        HeaderName::from(header.to_string())
    }
}

impl FromStr for HeaderValue {
    type Err = Error;
    fn from_str(value: &str) -> std::result::Result<Self, <Self as std::str::FromStr>::Err> {
        Ok(HeaderValue::from(value.to_owned()))
    }
}

impl From<String> for HeaderValue {
    fn from(string: String) -> Self {
        HeaderValue {
            inner: Bytes::from(string),
        }
    }
}

impl TryFrom<HeaderValue> for ReqwestHeaderValue {
    type Error = Error;
    fn try_from(
        value: HeaderValue,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderValue>>::Error> {
        Ok(ReqwestHeaderValue::from_bytes(&value.inner[..])?)
    }
}

impl From<ReqwestHeaderValue> for HeaderValue {
    fn from(value: ReqwestHeaderValue) -> Self {
        HeaderValue {
            inner: Bytes::copy_from_slice(value.as_bytes()),
        }
    }
}

impl From<&str> for HeaderValue {
    fn from(string: &str) -> Self {
        Self::from(string.to_owned())
    }
}

impl TryFrom<HeaderMap> for ReqwestHeaderMap {
    type Error = Error;
    fn try_from(
        headers: HeaderMap,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderMap>>::Error> {
        Ok(headers
            .into_iter()
            .map(|(k, v)| {
                (
                    ReqwestHeaderName::try_from(k).unwrap(),
                    ReqwestHeaderValue::try_from(v).unwrap(),
                )
            })
            .collect())
    }
}

impl TryFrom<HeaderMap> for WebSocketHeaderMap {
    type Error = Error;
    fn try_from(
        headers: HeaderMap,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderMap>>::Error> {
        let mut output = WebSocketHeaderMap::new();
        for (key, val) in headers {
            output.append_raw(key.to_string(), val.inner[..].to_vec());
        }
        Ok(output)
    }
}

impl IntoIterator for HeaderMap {
    type Item = (HeaderName, HeaderValue);
    type IntoIter = IntoIter;
    fn into_iter(self) -> <Self as std::iter::IntoIterator>::IntoIter {
        IntoIter {
            inner: self.map.into_iter(),
        }
    }
}

impl HeaderMap {
    pub fn new() -> Self {
        HeaderMap {
            map: HashMap::new(),
        }
    }

    pub fn insert<T: Into<HeaderName>, U: Into<HeaderValue>>(
        &mut self,
        key: T,
        value: U,
    ) -> Option<HeaderValue> {
        self.map.insert(key.into(), value.into())
    }
}

impl Default for HeaderMap {
    fn default() -> Self {
        Self::new()
    }
}

impl Iterator for IntoIter {
    type Item = (HeaderName, HeaderValue);
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        self.inner.next()
    }
}
