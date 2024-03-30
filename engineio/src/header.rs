use crate::Error;
use bytes::Bytes;
use http::{
    header::HeaderName as HttpHeaderName, HeaderMap as HttpHeaderMap,
    HeaderValue as HttpHeaderValue,
};
use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::str::FromStr;

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub struct HeaderName {
    inner: Box<str>,
}

#[derive(Eq, PartialEq, Hash, Debug, Clone)]
pub struct HeaderValue {
    inner: Bytes,
}

#[derive(Eq, PartialEq, Debug, Clone, Default)]
pub struct HeaderMap {
    map: HashMap<HeaderName, HeaderValue>,
}

pub struct IntoIter {
    inner: std::collections::hash_map::IntoIter<HeaderName, HeaderValue>,
}

impl Display for HeaderName {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(self.inner.as_ref())
    }
}

impl From<String> for HeaderName {
    fn from(string: String) -> Self {
        HeaderName {
            inner: string.into_boxed_str(),
        }
    }
}

impl TryFrom<HeaderName> for HttpHeaderName {
    type Error = Error;
    fn try_from(
        header: HeaderName,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderName>>::Error> {
        Ok(HttpHeaderName::from_str(header.inner.as_ref())?)
    }
}

impl From<HttpHeaderName> for HeaderName {
    fn from(header: HttpHeaderName) -> Self {
        HeaderName::from(header.to_string())
    }
}

impl From<String> for HeaderValue {
    fn from(string: String) -> Self {
        HeaderValue {
            inner: Bytes::from(string),
        }
    }
}

impl TryFrom<HeaderValue> for HttpHeaderValue {
    type Error = Error;
    fn try_from(
        value: HeaderValue,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderValue>>::Error> {
        Ok(HttpHeaderValue::from_bytes(&value.inner[..])?)
    }
}

impl From<HttpHeaderValue> for HeaderValue {
    fn from(value: HttpHeaderValue) -> Self {
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

impl TryFrom<HeaderMap> for HttpHeaderMap {
    type Error = Error;
    fn try_from(
        headers: HeaderMap,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<HeaderMap>>::Error> {
        headers
            .into_iter()
            .map(|(key, value)| {
                Ok((
                    HttpHeaderName::try_from(key)?,
                    HttpHeaderValue::try_from(value)?,
                ))
            })
            .collect()
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

impl Iterator for IntoIter {
    type Item = (HeaderName, HeaderValue);
    fn next(&mut self) -> std::option::Option<<Self as std::iter::Iterator>::Item> {
        self.inner.next()
    }
}
