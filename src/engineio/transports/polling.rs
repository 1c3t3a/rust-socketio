use crate::engineio::transports::Transport;
use crate::error::{Error, Result};
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use reqwest::{
    blocking::{Client, ClientBuilder},
    header::HeaderMap,
};
use std::sync::{Arc, Mutex};

pub(super) struct PollingTransport {
    client: Arc<Mutex<Client>>,
}

impl PollingTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(tls_config: Option<TlsConnector>, opening_headers: Option<HeaderMap>) -> Self {
        let client = match (tls_config.clone(), opening_headers.clone()) {
            (Some(config), Some(map)) => ClientBuilder::new()
                .use_preconfigured_tls(config)
                .default_headers(map)
                .build()
                .unwrap(),
            (Some(config), None) => ClientBuilder::new()
                .use_preconfigured_tls(config)
                .build()
                .unwrap(),
            (None, Some(map)) => ClientBuilder::new().default_headers(map).build().unwrap(),
            (None, None) => Client::new(),
        };

        PollingTransport {
            client: Arc::new(Mutex::new(client)),
        }
    }
}

impl Transport for PollingTransport {
    fn emit(&self, address: String, data: Bytes, is_binary_att: bool) -> Result<()> {
        let data_to_send = if is_binary_att {
            // the binary attachment gets `base64` encoded
            let mut packet_bytes = BytesMut::with_capacity(data.len() + 1);
            packet_bytes.put_u8(b'b');

            let encoded_data = base64::encode(data);
            packet_bytes.put(encoded_data.as_bytes());

            packet_bytes.freeze()
        } else {
            data
        };

        let client = self.client.lock()?;
        let status = client
            .post(address)
            .body(data_to_send)
            .send()?
            .status()
            .as_u16();

        drop(client);

        if status != 200 {
            let error = Error::HttpError(status);
            return Err(error);
        }

        Ok(())
    }

    fn poll(&self, address: String) -> Result<Bytes> {
        // we won't use the shared client as this blocks the resource
        // in the long polling requests
        Ok(Client::new().get(address).send().unwrap().bytes().unwrap())
    }
}
