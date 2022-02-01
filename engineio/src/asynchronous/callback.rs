use bytes::Bytes;
use futures_util::Future;
use std::{fmt::Debug, ops::Deref, pin::Pin, sync::Arc};

use crate::Packet;

pub(crate) type AsyncCallback<I> = dyn Fn(I) -> Pin<Box<dyn Future<Output = ()>>>;

#[derive(Clone)]
pub(crate) struct OptionalCallback<I> {
    inner: Arc<Option<Box<AsyncCallback<I>>>>,
}

impl<I> OptionalCallback<I> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: Fn(I) -> Pin<Box<dyn Future<Output = ()>>> + 'static,
    {
        OptionalCallback {
            inner: Arc::new(Some(Box::new(callback))),
        }
    }

    pub(crate) fn default() -> Self {
        OptionalCallback {
            inner: Arc::new(None),
        }
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for OptionalCallback<String> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.write_fmt(format_args!(
            "Callback({:?})",
            if self.inner.is_some() {
                "Fn(String)"
            } else {
                "None"
            }
        ))
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for OptionalCallback<()> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.write_fmt(format_args!(
            "Callback({:?})",
            if self.inner.is_some() {
                "Fn(())"
            } else {
                "None"
            }
        ))
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for OptionalCallback<Packet> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.write_fmt(format_args!(
            "Callback({:?})",
            if self.inner.is_some() {
                "Fn(Packet)"
            } else {
                "None"
            }
        ))
    }
}

#[cfg_attr(tarpaulin, ignore)]
impl Debug for OptionalCallback<Bytes> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        f.write_fmt(format_args!(
            "Callback({:?})",
            if self.inner.is_some() {
                "Fn(Bytes)"
            } else {
                "None"
            }
        ))
    }
}

impl<I> Deref for OptionalCallback<I> {
    type Target = Option<Box<AsyncCallback<I>>>;
    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        self.inner.as_ref()
    }
}
