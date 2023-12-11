use futures_util::future::BoxFuture;
use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::{Event, Payload};

use super::client::Client;

/// Internal type, provides a way to store futures and return them in a boxed manner.
pub(crate) type DynAsyncCallback = Box<
    dyn for<'a> FnMut(Payload, Client, Option<i32>) -> BoxFuture<'static, ()>
        + 'static
        + Send
        + Sync,
>;

pub(crate) type DynAsyncAnyCallback = Box<
    dyn for<'a> FnMut(Event, Payload, Client, Option<i32>) -> BoxFuture<'static, ()>
        + 'static
        + Send
        + Sync,
>;

pub(crate) struct Callback<T> {
    inner: T,
}

impl<T> Debug for Callback<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Callback")
    }
}

impl Deref for Callback<DynAsyncCallback> {
    type Target = dyn for<'a> FnMut(Payload, Client, Option<i32>) -> BoxFuture<'static, ()>
        + 'static
        + Sync
        + Send;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl DerefMut for Callback<DynAsyncCallback> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut()
    }
}

impl Callback<DynAsyncCallback> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: for<'a> FnMut(Payload, Client, Option<i32>) -> BoxFuture<'static, ()>
            + 'static
            + Sync
            + Send,
    {
        Callback {
            inner: Box::new(callback),
        }
    }
}

impl Deref for Callback<DynAsyncAnyCallback> {
    type Target = dyn for<'a> FnMut(Event, Payload, Client, Option<i32>) -> BoxFuture<'static, ()>
        + 'static
        + Sync
        + Send;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl DerefMut for Callback<DynAsyncAnyCallback> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut()
    }
}

impl Callback<DynAsyncAnyCallback> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: for<'a> FnMut(Event, Payload, Client, Option<i32>) -> BoxFuture<'static, ()>
            + 'static
            + Sync
            + Send,
    {
        Callback {
            inner: Box::new(callback),
        }
    }
}
