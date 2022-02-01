use futures_util::Future;
use std::{ops::Deref, pin::Pin, sync::Arc};

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

impl<I> Deref for OptionalCallback<I> {
    type Target = Option<Box<AsyncCallback<I>>>;
    fn deref(&self) -> &<Self as std::ops::Deref>::Target {
        self.inner.as_ref()
    }
}
