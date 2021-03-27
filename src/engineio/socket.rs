#![allow(unused)]
use std::thread;

use super::packet::{Packet, PacketId};
use crate::engineio::transport::TransportClient;
use crate::error::{Error, Result};
use bytes::Bytes;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

/// An `engine.io` socket which manages a connection with the server and allows
/// it to register common callbacks.
#[derive(Clone, Debug)]
pub struct EngineSocket {
    transport_client: Arc<RwLock<TransportClient>>,
    serving: Arc<AtomicBool>,
}

impl EngineSocket {
    /// Creates an instance of `EngineSocket`.
    pub fn new(engine_io_mode: bool) -> Self {
        EngineSocket {
            transport_client: Arc::new(RwLock::new(TransportClient::new(engine_io_mode))),
            serving: Arc::new(AtomicBool::default()),
        }
    }

    /// Binds the socket to a certain `address`. Attention! This doesn't allow
    /// to configure callbacks afterwards.
    pub fn bind<T: Into<String>>(&self, address: T) -> Result<()> {
        if self
            .transport_client
            .read()?
            .connected
            .load(Ordering::Relaxed)
        {
            return Err(Error::IllegalActionAfterOpen);
        }
        self.transport_client.write()?.open(address.into())?;

        let cl = Arc::clone(&self.transport_client);
        thread::spawn(move || {
            let mut s = cl.read().unwrap().clone();
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            loop {
                match s.poll_cycle() {
                    Ok(_) => break,
                    e @ Err(Error::HttpError(_)) | e @ Err(Error::ReqwestError(_)) => panic!(e),
                    _ => (),
                }
            }
        });
        self.serving.swap(true, Ordering::SeqCst);

        Ok(())
    }

    /// Sends a packet to the server.
    pub fn emit(&mut self, packet: Packet) -> Result<()> {
        if !self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.read()?.emit(packet, false)
    }

    /// Sends a socketio binary attachement to the server.
    pub fn emit_binary_attachement(&mut self, attachement: Bytes) -> Result<()> {
        if !self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        let packet = Packet::new(PacketId::Message, attachement);

        self.transport_client.read()?.emit(packet, true)
    }

    /// Registers the `on_open` callback.
    pub fn on_open<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::IllegalActionAfterOpen);
        }
        self.transport_client.write()?.set_on_open(function);
        Ok(())
    }

    /// Registers the `on_close` callback.
    pub fn on_close<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::IllegalActionAfterOpen);
        }
        self.transport_client.write()?.set_on_close(function);
        Ok(())
    }

    /// Registers the `on_packet` callback.
    pub fn on_packet<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::IllegalActionAfterOpen);
        }
        self.transport_client.write()?.set_on_packet(function);
        Ok(())
    }

    /// Registers the `on_data` callback.
    pub fn on_data<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(Bytes) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::IllegalActionAfterOpen);
        }
        self.transport_client.write()?.set_on_data(function);
        Ok(())
    }

    /// Registers the `on_error` callback.
    pub fn on_error<F>(&mut self, function: F) -> Result<()>
    where
        F: Fn(String) + 'static + Sync + Send + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::IllegalActionAfterOpen);
        }
        self.transport_client.write()?.set_on_error(function);
        Ok(())
    }
}

#[cfg(test)]
mod test {

    use std::{thread::sleep, time::Duration};

    use crate::engineio::packet::PacketId;

    use super::*;

    const SERVER_URL: &str = "http://localhost:4201";

    #[test]
    fn test_basic_connection() {
        let mut socket = EngineSocket::new(true);

        assert!(socket
            .on_open(|_| {
                println!("Open event!");
            })
            .is_ok());

        assert!(socket
            .on_packet(|packet| {
                println!("Received packet: {:?}", packet);
            })
            .is_ok());

        assert!(socket
            .on_data(|data| {
                println!("Received packet: {:?}", std::str::from_utf8(&data));
            })
            .is_ok());

        assert!(socket.bind(SERVER_URL).is_ok());

        assert!(socket
            .emit(Packet::new(
                PacketId::Message,
                Bytes::from_static(b"Hello World"),
            ))
            .is_ok());

        assert!(socket
            .emit(Packet::new(
                PacketId::Message,
                Bytes::from_static(b"Hello World2"),
            ))
            .is_ok());

        assert!(socket
            .emit(Packet::new(PacketId::Pong, Bytes::new()))
            .is_ok());

        sleep(Duration::from_secs(26));

        assert!(socket
            .emit(Packet::new(
                PacketId::Message,
                Bytes::from_static(b"Hello World3"),
            ))
            .is_ok());
    }

    #[test]
    fn test_illegal_actions() {
        let mut sut = EngineSocket::new(true);

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::from_static(b"")))
            .is_err());
        assert!(sut
            .emit_binary_attachement(Bytes::from_static(b""))
            .is_err());

        assert!(sut.bind(SERVER_URL).is_ok());

        assert!(sut.on_open(|_| {}).is_err());
        assert!(sut.on_close(|_| {}).is_err());
        assert!(sut.on_packet(|_| {}).is_err());
        assert!(sut.on_data(|_| {}).is_err());
        assert!(sut.on_error(|_| {}).is_err());
    }
}
