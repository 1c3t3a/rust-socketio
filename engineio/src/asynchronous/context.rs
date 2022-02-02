use crate::error::Result;
use std::sync::Arc;
use futures_util::Future;
use tokio::runtime::{Builder, Runtime};

/// A Context type that needs to be created once and is passed
/// to the [`crate::asynchronous::client::ClientBuilder`] while 
/// creatinga client. Provides means for executing the callbacks.
#[derive(Clone, Debug)]
pub struct Context {
    rt: Arc<Runtime>,
}

impl Context {
    pub fn new() -> Result<Context> {
        Ok(Context {
            rt: Arc::new(Builder::new_multi_thread().build()?),
        })
    }

    pub(crate) fn spawn_future<F: 'static + Future + Send>(&self, fut: F)
    where
        <F as Future>::Output: Send,
    {
        self.rt.spawn(fut);
    }
}