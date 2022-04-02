use std::pin::Pin;

use futures_util::Stream;

/// A generator is an internal type that represents a [`Send`] [`futures_util::Stream`]
/// that yields a certain type `T` whenever it's polled.
pub(crate) type Generator<T> = Pin<Box<dyn Stream<Item = T> + 'static + Send>>;
