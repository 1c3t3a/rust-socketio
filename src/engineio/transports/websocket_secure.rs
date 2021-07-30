use crate::engineio::packet::PacketId;
use crate::engineio::transport::Transport;
use crate::error::Result;
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use std::borrow::Cow;
use std::sync::{Arc, Mutex, RwLock};
use websocket::{
    client::sync::Client as WsClient,
    client::Url,
    dataframe::Opcode,
    header::Headers,
    sync::stream::{TcpStream, TlsStream},
    ws::dataframe::DataFrame,
    ClientBuilder as WsClientBuilder, Message,
};

pub(crate) struct WebsocketSecureTransport {
    client: Arc<Mutex<WsClient<TlsStream<TcpStream>>>>,
    base_url: Arc<RwLock<String>>,
}

impl WebsocketSecureTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(base_url: Url, tls_config: Option<TlsConnector>, headers: Option<Headers>) -> Self {
        let url = base_url
            .clone()
            .query_pairs_mut()
            .append_pair("transport", "websocket")
            .finish()
            .clone();
        let mut client_bulider = WsClientBuilder::new(base_url[..].as_ref()).unwrap();
        if let Some(headers) = headers {
            client_bulider = client_bulider.custom_headers(&headers);
        }
        let client = client_bulider.connect_secure(tls_config).unwrap();

        client.set_nonblocking(false).unwrap();

        WebsocketSecureTransport {
            client: Arc::new(Mutex::new(client)),
            base_url: Arc::new(RwLock::new(url.to_string())),
        }
    }
}

impl Transport for WebsocketSecureTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut writer = self.client.lock()?;

        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };
        writer.send_message(&message)?;

        Ok(())
    }

    fn poll(&self) -> Result<Bytes> {
        let mut receiver = self.client.lock()?;

        // if this is a binary payload, we mark it as a message
        let received_df = receiver.recv_dataframe()?;
        match received_df.opcode {
            Opcode::Binary => {
                let mut message = BytesMut::with_capacity(received_df.data.len() + 1);
                message.put_u8(PacketId::Message as u8);
                message.put(received_df.take_payload().as_ref());

                Ok(message.freeze())
            }
            _ => Ok(Bytes::from(received_df.take_payload())),
        }
    }

    fn base_url(&self) -> Result<String> {
        Ok(self.base_url.read()?.clone())
    }

    fn set_base_url(&self, url: String) -> Result<()> {
        *self.base_url.write()? = url;
        Ok(())
    }
}
/*
//TODO: implement unit tests for base_url and integration via engineio
#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;
    const SERVER_URL: &str = "http://localhost:4201";
    #[test]
    fn wss_transport_base_url() -> Result<()> {
        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());
        let transport = WebsocketSecureTransport::new(Url::from_str(&url[..]).unwrap(), None, None);
        assert_eq!(transport.base_url()?, url.clone() + "/?transport=websocket");
        transport.set_base_url("127.0.0.1".to_owned())?;
        // TODO: Change me to "127.0.0.1/?transport=websocket"
        assert_eq!(transport.base_url()?, "127.0.0.1");
        assert_ne!(transport.base_url()?, url);
        Ok(())
    }
}
*/
