use crate::engineio::packet::{Packet, PacketId};
use crate::engineio::transport::Transport;
use crate::error::{Error, Result};
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
    pub fn new(base_url: Url, tls_config: Option<TlsConnector>, headers: Headers) -> Self {
        let url = base_url
            .clone()
            .query_pairs_mut()
            .append_pair("transport", "websocket")
            .finish()
            .clone();
        let client = WsClientBuilder::new(base_url.to_string()[..].as_ref())
            .unwrap()
            .custom_headers(&headers)
            .connect_secure(tls_config)
            .unwrap();

        client.set_nonblocking(false).unwrap();

        WebsocketSecureTransport {
            client: Arc::new(Mutex::new(client)),
            base_url: Arc::new(RwLock::new(url.to_string())),
        }
    }

    pub(super) fn probe(&self) -> Result<()> {
        let mut client = self.client.lock()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        client.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Ping, Bytes::from("probe"))
                .encode()
                .as_ref(),
        )))?;

        // expect to receive a probe packet
        let message = client.recv_message()?;
        if message.take_payload() != Packet::new(PacketId::Pong, Bytes::from("probe")).encode() {
            return Err(Error::InvalidPacket());
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        client.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Upgrade, Bytes::from(""))
                .encode()
                .as_ref(),
        )))?;

        Ok(())
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
