use std::sync::Arc;

use bytes::Bytes;
use futures_util::future::BoxFuture;

use super::{server, Server, ServerOption};
use crate::{asynchronous::callback::OptionalCallback, Packet};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ServerBuilder {
    port: u16,
    on_error: OptionalCallback<String>,
    on_open: OptionalCallback<()>,
    on_close: OptionalCallback<()>,
    on_data: OptionalCallback<Bytes>,
    on_packet: OptionalCallback<Packet>,
    server_option: ServerOption,
}

#[allow(dead_code)]
impl ServerBuilder {
    pub fn new(port: u16) -> Self {
        Self {
            port,
            on_close: OptionalCallback::default(),
            on_data: OptionalCallback::default(),
            on_error: OptionalCallback::default(),
            on_open: OptionalCallback::default(),
            on_packet: OptionalCallback::default(),
            server_option: Default::default(),
        }
    }

    /// Registers the `on_close` callback.
    pub fn on_close<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(()) -> BoxFuture<'static, ()>,
    {
        self.on_close = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_data` callback.
    pub fn on_data<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(Bytes) -> BoxFuture<'static, ()>,
    {
        self.on_data = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_error` callback.
    pub fn on_error<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(String) -> BoxFuture<'static, ()>,
    {
        self.on_error = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_open` callback.
    pub fn on_open<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(()) -> BoxFuture<'static, ()>,
    {
        self.on_open = OptionalCallback::new(callback);
        self
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<T>(mut self, callback: T) -> Self
    where
        T: 'static + Send + Sync + Fn(Packet) -> BoxFuture<'static, ()>,
    {
        self.on_packet = OptionalCallback::new(callback);
        self
    }

    pub fn server_option(mut self, server_option: ServerOption) -> Self {
        self.server_option = server_option;
        self
    }

    pub fn build(self) -> Server {
        Server {
            inner: Arc::new(server::Inner {
                port: self.port,
                server_option: self.server_option,
                on_open: self.on_open,
                on_close: self.on_close,
                on_error: self.on_error,
                on_data: self.on_data,
                on_packet: self.on_packet,
                id_generator: Default::default(),
                sockets: Default::default(),
            }),
        }
    }
}
