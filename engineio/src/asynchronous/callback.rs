use bytes::Bytes;
use futures_util::future::BoxFuture;
use std::{fmt::Debug, ops::Deref, sync::Arc};

use crate::Packet;

/// Internal type, provides a way to store futures and return them in a boxed manner.
pub(crate) type DynAsyncCallback<I> = dyn 'static + Send + Sync + Fn(I) -> BoxFuture<'static, ()>;

/// Internal type, might hold an async callback.
#[derive(Clone)]
pub(crate) struct OptionalCallback<I> {
    inner: Option<Arc<DynAsyncCallback<I>>>,
}

impl<I> OptionalCallback<I> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(I) -> BoxFuture<'static, ()>,
    {
        OptionalCallback {
            inner: Some(Arc::new(callback)),
        }
    }

    pub(crate) fn default() -> Self {
        OptionalCallback { inner: None }
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
    type Target = Option<Arc<DynAsyncCallback<I>>>;
    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        &self.inner
    }
}
