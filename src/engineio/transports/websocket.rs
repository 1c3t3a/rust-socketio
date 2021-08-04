use crate::engineio::packet::Packet;
use crate::engineio::packet::PacketId;
use crate::engineio::transport::Transport;
use crate::error::{Error, Result};
use bytes::{BufMut, Bytes, BytesMut};
use std::borrow::Cow;
use std::sync::{Arc, Mutex, RwLock};
use websocket::{
    client::Url, dataframe::Opcode, header::Headers, receiver::Reader, sync::stream::TcpStream,
    sync::Writer, ws::dataframe::DataFrame, ClientBuilder as WsClientBuilder, Message,
};

#[derive(Clone)]
pub struct WebsocketTransport {
    sender: Arc<Mutex<Writer<TcpStream>>>,
    receiver: Arc<Mutex<Reader<TcpStream>>>,
    base_url: Arc<RwLock<url::Url>>,
}

impl WebsocketTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(base_url: Url, headers: Option<Headers>) -> Self {
        let mut url = base_url
            .clone()
            .query_pairs_mut()
            .append_pair("transport", "websocket")
            .finish()
            .clone();
        url.set_scheme("ws").unwrap();
        let mut client_builder = WsClientBuilder::new(url[..].as_ref()).unwrap();
        if let Some(headers) = headers {
            client_builder = client_builder.custom_headers(&headers);
        }
        let client = client_builder.connect_insecure().unwrap();

        client.set_nonblocking(false).unwrap();

        let (receiver, sender) = client.split().unwrap();

        WebsocketTransport {
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
            // SAFTEY: already a URL parsing can't fail
            base_url: Arc::new(RwLock::new(url::Url::parse(&url.to_string()).unwrap())),
        }
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) fn upgrade(&self) -> Result<()> {
        let mut sender = self.sender.lock()?;
        let mut receiver = self.receiver.lock()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        sender.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Ping, Bytes::from("probe"))
                .encode_packet()
                .as_ref(),
        )))?;

        // expect to receive a probe packet
        let message = receiver.recv_message()?;
        if message.take_payload()
            != Packet::new(PacketId::Pong, Bytes::from("probe")).encode_packet()
        {
            return Err(Error::InvalidPacket());
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        sender.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Upgrade, Bytes::from(""))
                .encode_packet()
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

    fn base_url(&self) -> Result<url::Url> {
        Ok(self.base_url.read()?.clone())
    }

    fn set_base_url(&self, url: url::Url) -> Result<()> {
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
    fn ws_transport_base_url() -> Result<()> {
        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned())
            + "/engine.io/?EIO=4";
        let transport = WebsocketTransport::new(
            Url::from_str(&url.replace("http://", "ws://")[..]).unwrap(),
            None,
        );
        assert_eq!(transport.base_url()?, url.clone() + "?transport=websocket");
        transport.set_base_url("127.0.0.1".to_owned())?;
        // TODO: Change me to "127.0.0.1/?transport=websocket"
        assert_eq!(transport.base_url()?, "127.0.0.1");
        assert_ne!(transport.base_url()?, url);
        Ok(())
    }
}
*/
