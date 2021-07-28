use crate::engineio::transport::Transport;
use crate::error::{Error, Result};
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use reqwest::Url;
use reqwest::{
    blocking::{Client, ClientBuilder},
    header::HeaderMap,
};
use std::sync::{Arc, Mutex, RwLock};

pub(crate) struct PollingTransport {
    client: Arc<Mutex<Client>>,
    base_url: Arc<RwLock<String>>,
}

impl PollingTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(
        base_url: Url,
        tls_config: Option<TlsConnector>,
        opening_headers: Option<HeaderMap>,
    ) -> Self {
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

        let url = base_url
            .clone()
            .query_pairs_mut()
            .append_pair("transport", "polling")
            .finish()
            .clone();

        PollingTransport {
            client: Arc::new(Mutex::new(client)),
            base_url: Arc::new(RwLock::new(url.to_string())),
        }
    }
}

impl Transport for PollingTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
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
            .post(self.address()?)
            .body(data_to_send)
            .send()?
            .status()
            .as_u16();

        drop(client);

        if status != 200 {
            let error = Error::IncompleteHttp(status);
            return Err(error);
        }

        Ok(())
    }

    fn poll(&self) -> Result<Bytes> {
        Ok(Client::new().get(self.address()?).send()?.bytes()?)
    }

    fn base_url(&self) -> Result<String> {
        Ok(self.base_url.read()?.clone())
    }

    fn set_base_url(&self, base_url: String) -> Result<()> {
        *self.base_url.write()? = base_url;
        Ok(())
    }
}
