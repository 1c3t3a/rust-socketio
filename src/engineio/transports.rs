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
use std::sync::{Arc, Mutex, RwLock};

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

enum TransportTypes {
    WebsocketSecure,
    Websocket,
    Polling,
}

pub struct Transports {
    websocket_secure: Arc<Mutex<Option<WebsocketSecureTransport>>>,
    websocket: Arc<Mutex<Option<WebsocketTransport>>>,
    polling: Arc<Mutex<Option<PollingTransport>>>,
    tls_config: Arc<Mutex<Option<TlsConnector>>>,
    transport_type: Arc<RwLock<TransportTypes>>,
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
            transport_type: Arc::new(RwLock::new(TransportTypes::Polling)),
        }
    }

    pub(super) fn upgrade_websocket(&mut self, address: String) -> Result<()> {
        if self.websocket_secure.lock()?.is_none() {
            *self.websocket.lock()? = Some(WebsocketTransport::new(address));
            self.websocket.lock()?.as_ref().unwrap().probe()?;
            *self.transport_type.write()? = TransportTypes::Websocket;
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
            *self.transport_type.write()? = TransportTypes::WebsocketSecure;
            Ok(())
        } else {
            Err(Error::TransportExists())
        }
    }
}

impl Transport for Transports {
    fn emit(&self, address: String, data: Bytes, is_binary_att: bool) -> Result<()> {
        match &*self.transport_type.read()? {
            TransportTypes::Websocket => {
                self.websocket
                    .lock()?
                    .as_ref()
                    .unwrap()
                    .emit(address, data, is_binary_att)
            }
            TransportTypes::WebsocketSecure => self
                .websocket_secure
                .lock()?
                .as_ref()
                .unwrap()
                .emit(address, data, is_binary_att),
            TransportTypes::Polling => {
                self.polling
                    .lock()?
                    .as_ref()
                    .unwrap()
                    .emit(address, data, is_binary_att)
            }
        }
    }

    fn poll(&mut self, address: String) -> Result<Bytes> {
        match &*self.transport_type.read()? {
            TransportTypes::Websocket => self.websocket.lock()?.as_mut().unwrap().poll(address),
            TransportTypes::WebsocketSecure => self
                .websocket_secure
                .lock()?
                .as_mut()
                .unwrap()
                .poll(address),
            TransportTypes::Polling => self.polling.lock()?.as_mut().unwrap().poll(address),
        }
    }

    fn name(&self) -> Result<String> {
        match &*self.transport_type.read()? {
            TransportTypes::Websocket => self.websocket_secure.lock()?.as_ref().unwrap().name(),
            TransportTypes::WebsocketSecure => self.websocket.lock()?.as_ref().unwrap().name(),
            TransportTypes::Polling => self.polling.lock()?.as_ref().unwrap().name(),
        }
    }
}
