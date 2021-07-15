mod polling;
mod websocket;
mod websocket_secure;
use self::polling::PollingTransport;
use self::websocket::WebsocketTransport;
use self::websocket_secure::WebsocketSecureTransport;
use crate::error::{Error, Result};
use bytes::Bytes;
use native_tls::TlsConnector;
use reqwest::header::HeaderMap;
use std::sync::{Arc, Mutex};

pub trait Transport {
    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    fn emit(&self, address: String, data: Bytes, is_binary_att: bool) -> Result<()>;

    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    fn poll(&mut self, address: String) -> Result<Bytes>;

    /// Gets the name of the transport type
    fn name(&self) -> Result<String>;
}

pub struct Transports {
    websocket_secure: Arc<Mutex<Option<WebsocketSecureTransport>>>,
    websocket: Arc<Mutex<Option<WebsocketTransport>>>,
    polling: Arc<Mutex<Option<PollingTransport>>>,
    tls_config: Arc<Mutex<Option<TlsConnector>>>,
}

impl Transports {
    pub fn new(tls_config: Option<TlsConnector>, opening_headers: Option<HeaderMap>) -> Self {
        Transports {
            websocket_secure: Arc::new(Mutex::new(None)),
            websocket: Arc::new(Mutex::new(None)),
            polling: Arc::new(Mutex::new(Some(PollingTransport::new(
                tls_config.clone(),
                opening_headers,
            )))),
            tls_config: Arc::new(Mutex::new(tls_config)),
        }
    }

    pub(super) fn upgrade_websocket(&mut self, address: String) -> Result<()> {
        if self.websocket_secure.lock()?.is_none() {
            *self.websocket.lock()? = Some(WebsocketTransport::new(address));
            self.websocket.lock()?.as_ref().unwrap().probe()?;
            Ok(())
        } else {
            Err(Error::TransportExists())
        }
    }

    pub(super) fn upgrade_websocket_secure(&mut self, address: String) -> Result<()> {
        if self.websocket_secure.lock()?.is_none() {
            *self.websocket_secure.lock()? = Some(WebsocketSecureTransport::new(
                address,
                self.tls_config.lock()?.clone(),
            ));
            self.websocket_secure.lock()?.as_ref().unwrap().probe()?;
            Ok(())
        } else {
            Err(Error::TransportExists())
        }
    }
}

impl Transport for Transports {
    fn emit(&self, address: String, data: Bytes, is_binary_att: bool) -> Result<()> {
        if self.websocket_secure.lock()?.is_some() {
            self.websocket_secure
                .lock()?
                .as_ref()
                .unwrap()
                .emit(address, data, is_binary_att)
        } else if self.websocket.lock()?.is_some() {
            self.websocket
                .lock()?
                .as_ref()
                .unwrap()
                .emit(address, data, is_binary_att)
        } else if self.polling.lock()?.is_some() {
            self.polling
                .lock()?
                .as_ref()
                .unwrap()
                .emit(address, data, is_binary_att)
        } else {
            Err(Error::NoTransport())
        }
    }

    fn poll(&mut self, address: String) -> Result<Bytes> {
        if self.websocket_secure.lock()?.is_some() {
            self.websocket_secure
                .lock()?
                .as_mut()
                .unwrap()
                .poll(address)
        } else if self.websocket.lock()?.is_some() {
            self.websocket.lock()?.as_mut().unwrap().poll(address)
        } else if self.polling.lock()?.is_some() {
            self.polling.lock()?.as_mut().unwrap().poll(address)
        } else {
            Err(Error::NoTransport())
        }
    }

    fn name(&self) -> Result<String> {
        if self.websocket_secure.lock()?.is_some() {
            self.websocket_secure.lock()?.as_ref().unwrap().name()
        } else if self.websocket.lock()?.is_some() {
            self.websocket.lock()?.as_ref().unwrap().name()
        } else if self.polling.lock()?.is_some() {
            self.polling.lock()?.as_ref().unwrap().name()
        } else {
            Err(Error::NoTransport())
        }
    }
}
