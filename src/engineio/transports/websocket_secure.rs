use crate::engineio::packet::Packet;
use crate::engineio::packet::PacketId;
use crate::engineio::transport::Transport;
use crate::error::Error;
use crate::error::Result;
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use std::borrow::Cow;
use std::convert::TryFrom;
use std::str::from_utf8;
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

#[derive(Clone)]
pub struct WebsocketSecureTransport {
    client: Arc<Mutex<WsClient<TlsStream<TcpStream>>>>,
    base_url: Arc<RwLock<url::Url>>,
}

impl WebsocketSecureTransport {
    /// Creates an instance of `WebsocketSecureTransport`.
    pub fn new(base_url: Url, tls_config: Option<TlsConnector>, headers: Option<Headers>) -> Self {
        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();
        let mut client_builder = WsClientBuilder::new(url[..].as_ref()).unwrap();
        if let Some(headers) = headers {
            client_builder = client_builder.custom_headers(&headers);
        }
        let client = client_builder.connect_secure(tls_config).unwrap();

        client.set_nonblocking(false).unwrap();

        WebsocketSecureTransport {
            client: Arc::new(Mutex::new(client)),
            // SAFTEY: already a URL parsing can't fail
            base_url: Arc::new(RwLock::new(url::Url::parse(&url.to_string()).unwrap())),
        }
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) fn upgrade(&self) -> Result<()> {
        let mut client = self.client.lock()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        client.send_message(&Message::text(Cow::Borrowed(from_utf8(&Bytes::from(
            Packet::new(PacketId::Ping, Bytes::from("probe")),
        ))?)))?;

        // expect to receive a probe packet
        let message = client.recv_message()?;
        let payload = message.take_payload();
        if Packet::try_from(Bytes::from(payload))?
            != Packet::new(PacketId::Pong, Bytes::from("probe"))
        {
            return Err(Error::InvalidPacket());
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        client.send_message(&Message::text(Cow::Borrowed(from_utf8(&Bytes::from(
            Packet::new(PacketId::Upgrade, Bytes::from("")),
        ))?)))?;

        Ok(())
    }
}

impl Transport for WebsocketSecureTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };

        let mut writer = self.client.lock()?;
        writer.send_message(&message)?;
        drop(writer);

        Ok(())
    }

    fn poll(&self) -> Result<Bytes> {
        let mut receiver = self.client.lock()?;

        // if this is a binary payload, we mark it as a message
        let received_df = receiver.recv_dataframe()?;
        drop(receiver);
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

    fn base_url(&self) -> Result<url::Url> {
        Ok(self.base_url.read()?.clone())
    }

    fn set_base_url(&self, url: url::Url) -> Result<()> {
        let mut url = url;
        if !url
            .query_pairs()
            .any(|(k, v)| k == "transport" && v == "websocket")
        {
            url.query_pairs_mut().append_pair("transport", "websocket");
        }
        url.set_scheme("wss").unwrap();
        *self.base_url.write()? = url;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::str::FromStr;
    #[test]
    fn websocket_secure_transport_base_url() -> Result<()> {
        let url =
            crate::engineio::test::engine_io_server_secure()?.to_string() + "engine.io/?EIO=4";
        let transport = WebsocketSecureTransport::new(
            Url::from_str(&url[..]).unwrap(),
            Some(crate::test::tls_connector()?),
            None,
        );
        let mut url = crate::engineio::test::engine_io_server_secure()?;
        url.set_path("/engine.io/");
        url.query_pairs_mut()
            .append_pair("EIO", "4")
            .append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();
        assert_eq!(transport.base_url()?.to_string(), url.to_string());
        transport.set_base_url(reqwest::Url::parse("https://127.0.0.1")?)?;
        assert_eq!(
            transport.base_url()?.to_string(),
            "wss://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url()?.to_string(), url.to_string());

        transport.set_base_url(reqwest::Url::parse(
            "http://127.0.0.1/?transport=websocket",
        )?)?;
        assert_eq!(
            transport.base_url()?.to_string(),
            "wss://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url()?.to_string(), url.to_string());
        Ok(())
    }
}
