use futures_util::future::BoxFuture;
use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::{asynchronous::ack::AckId, Payload};

/// Internal type, provides a way to store futures and return them in a boxed manner.
type DynAsyncCallback<C> = Box<
    dyn for<'a> FnMut(Payload, C, Option<AckId>) -> BoxFuture<'static, ()> + 'static + Send + Sync,
>;

pub(crate) struct Callback<C> {
    inner: DynAsyncCallback<C>,
}

impl<C> Debug for Callback<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Callback")
    }
}

impl<C> Deref for Callback<C> {
    type Target = dyn for<'a> FnMut(Payload, C, Option<AckId>) -> BoxFuture<'static, ()>
        + 'static
        + Sync
        + Send;

    fn deref(&self) -> &Self::Target {
        self.inner.as_ref()
    }
}

impl<C> DerefMut for Callback<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.inner.as_mut()
    }
}

impl<C> Callback<C> {
    pub(crate) fn new<T>(callback: T) -> Self
    where
        T: for<'a> FnMut(Payload, C, Option<AckId>) -> BoxFuture<'static, ()>
            + 'static
            + Sync
            + Send,
    {
        Callback {
            inner: Box::new(callback),
        }
    }
}
