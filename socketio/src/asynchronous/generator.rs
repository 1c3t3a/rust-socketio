use std::pin::Pin;

use futures_util::Stream;

pub(crate) type Generator<T> = Pin<Box<dyn Stream<Item = T> + 'static + Send>>;
