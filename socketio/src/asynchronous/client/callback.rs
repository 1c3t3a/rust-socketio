use futures_util::future::BoxFuture;
use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::Payload;

use super::client::Client;

/// Internal type, provides a way to store futures and return them in a boxed manner.
type DynAsyncCallback =
    Box<dyn for<'a> FnMut(Payload, Client) -> BoxFuture<'static, ()> + 'static + Send + Sync>;

pub(crate) struct Callback {
    inner: DynAsyncCallback,
}

impl Debug for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Callback")
    }
}

impl Deref for Callback {
    type Target =
        dyn for<'a> FnMut(Payload, Client) -> BoxFuture<'static, ()> + 'static + Sync + Send;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl DerefMut for Callback {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut()
    }
}

impl Callback {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: for<'a> FnMut(Payload, Client) -> BoxFuture<'static, ()> + 'static + Sync + Send,
    {
        Callback {
            inner: Box::new(callback),
        }
    }
}
