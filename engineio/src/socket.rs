use crate::transport::TransportType;

use crate::error::{Error, Result};
use crate::packet::{HandshakePacket, Packet, PacketId, Payload};
use bytes::Bytes;
use std::convert::TryFrom;
use std::{fmt::Debug, sync::atomic::Ordering};
use std::{
    sync::{atomic::AtomicBool, Arc, Mutex, RwLock},
    time::Instant,
};

/// Type of a `Callback` function. (Normal closures can be passed in here).
type Callback<I> = Arc<RwLock<Option<Box<dyn Fn(I) + 'static + Sync + Send>>>>;

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone)]
pub struct Socket {
    transport: Arc<TransportType>,
    //TODO: Store these in a map
    pub on_error: Callback<String>,
    pub on_open: Callback<()>,
    pub on_close: Callback<()>,
    pub on_data: Callback<Bytes>,
    pub on_packet: Callback<Packet>,
    pub connected: Arc<AtomicBool>,
    last_ping: Arc<Mutex<Instant>>,
    last_pong: Arc<Mutex<Instant>>,
    connection_data: Arc<HandshakePacket>,
}

impl Socket {
    pub(crate) fn new(transport: TransportType, handshake: HandshakePacket) -> Self {
        Socket {
            on_error: Arc::new(RwLock::new(None)),
            on_open: Arc::new(RwLock::new(None)),
            on_close: Arc::new(RwLock::new(None)),
            on_data: Arc::new(RwLock::new(None)),
            on_packet: Arc::new(RwLock::new(None)),
            transport: Arc::new(transport),
            connected: Arc::new(AtomicBool::default()),
            last_ping: Arc::new(Mutex::new(Instant::now())),
            last_pong: Arc::new(Mutex::new(Instant::now())),
            connection_data: Arc::new(handshake),
        }
    }

    /// Registers the `on_open` callback.
    pub fn on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_open = self.on_open.write()?;
        *on_open = Some(Box::new(function));
        drop(on_open);
        Ok(())
    }

    /// Registers the `on_error` callback.
    pub fn on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_error = self.on_error.write()?;
        *on_error = Some(Box::new(function));
        drop(on_error);
        Ok(())
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_packet = self.on_packet.write()?;
        *on_packet = Some(Box::new(function));
        drop(on_packet);
        Ok(())
    }

    /// Registers the `on_data` callback.
    pub fn on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_data = self.on_data.write()?;
        *on_data = Some(Box::new(function));
        drop(on_data);
        Ok(())
    }

    /// Registers the `on_close` callback.
    pub fn on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.is_connected()? {
            return Err(Error::IllegalActionAfterOpen());
        }
        let mut on_close = self.on_close.write()?;
        *on_close = Some(Box::new(function));
        drop(on_close);
        Ok(())
    }

    pub fn close(&self) -> Result<()> {
        self.emit(Packet::new(PacketId::Close, Bytes::new()))?;
        self.connected.store(false, Ordering::Release);
        Ok(())
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub fn connect(&self) -> Result<()> {
        // SAFETY: Has valid handshake due to type
        self.connected.store(true, Ordering::Release);

        if let Some(on_open) = self.on_open.read()?.as_ref() {
            spawn_scoped!(on_open(()));
        }

        // set the last ping to now and set the connected state
        *self.last_ping.lock()? = Instant::now();

        // emit a pong packet to keep trigger the ping cycle on the server
        self.emit(Packet::new(PacketId::Pong, Bytes::new()))?;

        Ok(())
    }

    /// Sends a packet to the server.
    pub fn emit(&self, packet: Packet) -> Result<()> {
        if !self.connected.load(Ordering::Acquire) {
            let error = Error::IllegalActionBeforeOpen();
            self.call_error_callback(format!("{}", error))?;
            return Err(error);
        }

        let is_binary = packet.packet_id == PacketId::MessageBinary;

        // send a post request with the encoded payload as body
        // if this is a binary attachment, then send the raw bytes
        let data: Bytes = if is_binary {
            packet.data
        } else {
            packet.into()
        };

        if let Err(error) = self.transport.as_transport().emit(data, is_binary) {
            self.call_error_callback(error.to_string())?;
            return Err(error);
        }

        Ok(())
    }

    /// Polls for next payload
    pub(crate) fn poll(&self) -> Result<Option<Payload>> {
        if self.connected.load(Ordering::Acquire) {
            let data = self.transport.as_transport().poll()?;

            if data.is_empty() {
                return Ok(None);
            }

            Ok(Some(Payload::try_from(data)?))
        } else {
            Err(Error::IllegalActionBeforeOpen())
        }
    }

    /// Calls the error callback with a given message.
    #[inline]
    fn call_error_callback(&self, text: String) -> Result<()> {
        let function = self.on_error.read()?;
        if let Some(function) = function.as_ref() {
            spawn_scoped!(function(text));
        }
        drop(function);

        Ok(())
    }

    // Check if the underlying transport client is connected.
    pub(crate) fn is_connected(&self) -> Result<bool> {
        Ok(self.connected.load(Ordering::Acquire))
    }

    pub(crate) fn pinged(&self) -> Result<()> {
        *self.last_ping.lock()? = Instant::now();
        Ok(())
    }

    pub(crate) fn handle_packet(&self, packet: Packet) -> Result<()> {
        if let Some(on_packet) = self.on_packet.read()?.as_ref() {
            spawn_scoped!(on_packet(packet));
        }
        Ok(())
    }

    pub(crate) fn handle_data(&self, data: Bytes) -> Result<()> {
        if let Some(on_data) = self.on_data.read()?.as_ref() {
            spawn_scoped!(on_data(data));
        }
        Ok(())
    }

    pub(crate) fn handle_close(&self) -> Result<()> {
        if let Some(on_close) = self.on_close.read()?.as_ref() {
            spawn_scoped!(on_close(()));
        }
        Ok(())
    }
}

impl Debug for Socket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "EngineSocket(transport: {:?}, on_error: {:?}, on_open: {:?}, on_close: {:?}, on_packet: {:?}, on_data: {:?}, connected: {:?}, last_ping: {:?}, last_pong: {:?}, connection_data: {:?})",
            self.transport,
            if self.on_error.read().unwrap().is_some() {
                "Fn(String)"
            } else {
                "None"
            },
            if self.on_open.read().unwrap().is_some() {
                "Fn(())"
            } else {
                "None"
            },
            if self.on_close.read().unwrap().is_some() {
                "Fn(())"
            } else {
                "None"
            },
            if self.on_packet.read().unwrap().is_some() {
                "Fn(Packet)"
            } else {
                "None"
            },
            if self.on_data.read().unwrap().is_some() {
                "Fn(Bytes)"
            } else {
                "None"
            },
            self.connected,
            self.last_ping,
            self.last_pong,
            self.connection_data,
        ))
    }
}
