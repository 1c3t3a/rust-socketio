use crate::Packet;
use bytes::Bytes;
use std::fmt::Debug;
use std::ops::Deref;
use std::sync::Arc;

pub(crate) type Callback<I> = dyn Fn(I) + 'static + Sync + Send;

#[derive(Clone)]
/// Internal type, only implements debug on fixed set of generics
pub(crate) struct OptionalCallback<I> {
    inner: Arc<Option<Box<Callback<I>>>>,
}

impl<I> OptionalCallback<I> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: Fn(I) + 'static + Sync + Send,
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
    type Target = Option<Box<Callback<I>>>;
    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        self.inner.as_ref()
    }
}
