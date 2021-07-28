use crate::engineio::packet::{Packet, PacketId};
use crate::engineio::transport::Transport;
use crate::error::{Error, Result};
use bytes::{BufMut, Bytes, BytesMut};
use std::borrow::Cow;
use std::sync::{Arc, Mutex, RwLock};
use websocket::{
    client::Url, dataframe::Opcode, header::Headers, receiver::Reader, sync::stream::TcpStream,
    sync::Writer, ws::dataframe::DataFrame, ClientBuilder as WsClientBuilder, Message,
};

pub(crate) struct WebsocketTransport {
    sender: Arc<Mutex<Writer<TcpStream>>>,
    receiver: Arc<Mutex<Reader<TcpStream>>>,
    base_url: Arc<RwLock<String>>,
}

impl WebsocketTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(base_url: Url, headers: Headers) -> Self {
        let url = base_url
            .clone()
            .query_pairs_mut()
            .append_pair("transport", "websocket")
            .finish()
            .clone();
        let client = WsClientBuilder::new(base_url[..].as_ref())
            .unwrap()
            .custom_headers(&headers)
            .connect_insecure()
            .unwrap();

        client.set_nonblocking(false).unwrap();

        let (receiver, sender) = client.split().unwrap();

        WebsocketTransport {
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
            base_url: Arc::new(RwLock::new(url.to_string())),
        }
    }

    pub(super) fn probe(&self) -> Result<()> {
        let mut sender = self.sender.lock()?;
        let mut receiver = self.receiver.lock()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        sender.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Ping, Bytes::from("probe"))
                .encode()
                .as_ref(),
        )))?;

        // expect to receive a probe packet
        let message = receiver.recv_message()?;
        if message.take_payload() != Packet::new(PacketId::Pong, Bytes::from("probe")).encode() {
            return Err(Error::InvalidPacket());
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        sender.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Upgrade, Bytes::from(""))
                .encode()
                .as_ref(),
        )))?;

        Ok(())
    }
}

impl Transport for WebsocketTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut sender = self.sender.lock()?;

        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };
        sender.send_message(&message)?;

        Ok(())
    }

    fn poll(&self) -> Result<Bytes> {
        let mut receiver = self.receiver.lock()?;

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
