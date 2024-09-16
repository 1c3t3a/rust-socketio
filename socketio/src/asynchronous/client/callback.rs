use futures_util::{future::BoxFuture, FutureExt};
use std::{
    fmt::Debug,
    future::Future,
    ops::{Deref, DerefMut},
};

use crate::{Event, Payload};

use super::client::{Client, ReconnectSettings};

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

pub(crate) type DynAsyncReconnectSettingsCallback =
    Box<dyn for<'a> FnMut() -> BoxFuture<'static, ReconnectSettings> + 'static + Send + Sync>;

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
    pub(crate) fn new_with_ack<T>(mut callback: T) -> Self
    where
        T: for<'a> FnMut(Payload, Client, i32) -> BoxFuture<'static, ()> + 'static + Sync + Send,
    {
        Callback {
            inner: Box::new(move |p, c, a| match a {
                Some(a) => callback(p, c, a).boxed(),
                None => std::future::ready(()).boxed(),
            }),
        }
    }

    pub(crate) fn new<T, Fut>(mut callback: T) -> Self
    where
        T: FnMut(Payload, Client) -> Fut + Sync + Send + 'static,
        Fut: Future<Output = ()> + 'static + Send,
    {
        Callback {
            inner: Box::new(move |p, c, _a| callback(p, c).boxed()),
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
    pub(crate) fn new_with_ack<T>(mut callback: T) -> Self
    where
        T: for<'a> FnMut(Event, Payload, Client, i32) -> BoxFuture<'static, ()>
            + 'static
            + Sync
            + Send,
    {
        Callback {
            inner: Box::new(move |e, p, c, a| match a {
                Some(a) => callback(e, p, c, a).boxed(),
                None => std::future::ready(()).boxed(),
            }),
        }
    }

    pub(crate) fn new<T, Fut>(mut callback: T) -> Self
    where
        T: FnMut(Event, Payload, Client) -> Fut + Sync + Send + 'static,
        Fut: Future<Output = ()> + 'static + Send,
    {
        Callback {
            inner: Box::new(move |e, p, c, _a| callback(e, p, c).boxed()),
        }
    }
}

impl Deref for Callback<DynAsyncReconnectSettingsCallback> {
    type Target =
        dyn for<'a> FnMut() -> BoxFuture<'static, ReconnectSettings> + 'static + Sync + Send;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl DerefMut for Callback<DynAsyncReconnectSettingsCallback> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut()
    }
}

impl Callback<DynAsyncReconnectSettingsCallback> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: for<'a> FnMut() -> BoxFuture<'static, ReconnectSettings> + 'static + Sync + Send,
    {
        Callback {
            inner: Box::new(callback),
        }
    }
}
