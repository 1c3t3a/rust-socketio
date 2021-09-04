use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use crate::{Payload, Socket};

type InnerCallback = Box<dyn for<'a> FnMut(Payload, &'a Socket) + 'static + Sync + Send>;

pub(crate) struct Callback {
    inner: InnerCallback,
}

impl Debug for Callback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Callback")
    }
}

impl Deref for Callback {
    type Target = dyn for<'a> FnMut(Payload, &'a Socket) + 'static + Sync + Send;

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
        T: for<'a> FnMut(Payload, &'a Socket) + 'static + Sync + Send,
    {
        Callback {
            inner: Box::new(callback),
        }
    }
}
