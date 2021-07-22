use crate::engineio::packet::Packet;
use crate::engineio::transports::Transport;
use crate::engineio::transports::Transports;
use crate::error::Result;
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::sync::{Arc, RwLock};

/// Type of a `Callback` function. (Normal closures can be passed in here).
type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I) + 'static + Sync + Send>>>>;

pub type TransportEmitter = TransportEmitterGeneric<Transports>;

/// A client which handles the plain transmission of packets in the `engine.io`
/// protocol. Used by the wrapper `EngineSocket`. This struct also holds the
/// callback functions.
#[derive(Clone)]
pub struct TransportEmitterGeneric<T: Transport> {
    transport: Arc<RwLock<T>>,
    pub on_error: Callback<String>,
    pub on_open: Callback<()>,
    pub on_close: Callback<()>,
    pub on_data: Callback<Bytes>,
    pub on_packet: Callback<Packet>,
}

/// Data which gets exchanged in a handshake as defined by the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
struct HandshakeData {
    sid: String,
    upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    ping_interval: u64,
    #[serde(rename = "pingTimeout")]
    ping_timeout: u64,
}

pub trait EventEmitter {
    /// Registers an `on_open` callback
    fn set_on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send;

    /// Registers an `on_error` callback.
    fn set_on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send;

    /// Registers an `on_packet` callback.
    fn set_on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send;

    /// Registers an `on_data` callback.
    fn set_on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send;

    /// Registers an `on_close` callback.
    fn set_on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send;
}

impl<T: Transport> TransportEmitterGeneric<T> {
    /// Creates an instance of `Transport`.
    pub fn new(transport: T) -> Self {
        TransportEmitterGeneric::<T> {
            transport: Arc::new(RwLock::new(transport)),
            on_error: Arc::new(RwLock::new(None)),
            on_open: Arc::new(RwLock::new(None)),
            on_close: Arc::new(RwLock::new(None)),
            on_data: Arc::new(RwLock::new(None)),
            on_packet: Arc::new(RwLock::new(None)),
        }
    }
}

impl TransportEmitterGeneric<Transports> {
    pub(super) fn upgrade_websocket(&mut self, address: String) -> Result<()> {
        self.transport.write()?.upgrade_websocket(address)
    }

    pub(super) fn upgrade_websocket_secure(&mut self, address: String) -> Result<()> {
        self.transport.write()?.upgrade_websocket_secure(address)
    }

    pub(super) fn get_transport_name(&self) -> Result<&'static str> {
        self.transport.write()?.get_transport_name()
    }
}

impl<T: Transport> EventEmitter for TransportEmitterGeneric<T> {
    /// Registers an `on_open` callback.
    fn set_on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        let mut on_open = self.on_open.write()?;
        *on_open = Some(Box::new(function));
        drop(on_open);
        Ok(())
    }

    /// Registers an `on_error` callback.
    fn set_on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        let mut on_error = self.on_error.write()?;
        *on_error = Some(Box::new(function));
        drop(on_error);
        Ok(())
    }

    /// Registers an `on_packet` callback.
    fn set_on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        let mut on_packet = self.on_packet.write()?;
        *on_packet = Some(Box::new(function));
        drop(on_packet);
        Ok(())
    }

    /// Registers an `on_data` callback.
    fn set_on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send,
    {
        let mut on_data = self.on_data.write()?;
        *on_data = Some(Box::new(function));
        drop(on_data);
        Ok(())
    }

    /// Registers an `on_close` callback.
    fn set_on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        let mut on_close = self.on_close.write()?;
        *on_close = Some(Box::new(function));
        drop(on_close);
        Ok(())
    }
}

impl<T: Transport> Transport for TransportEmitterGeneric<T> {
    fn emit(&self, address: String, data: Bytes, is_binary_att: bool) -> Result<()> {
        self.transport.read()?.emit(address, data, is_binary_att)
    }

    fn poll(&self, address: String) -> Result<Bytes> {
        self.transport.read()?.poll(address)
    }
}

impl Debug for TransportEmitterGeneric<Transports> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "TransportType({})",
            self.transport.read().unwrap().get_transport_name().unwrap()
        ))
    }
}
