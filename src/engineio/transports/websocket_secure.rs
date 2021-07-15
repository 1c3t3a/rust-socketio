use crate::engineio::packet::{Packet, PacketId};
use crate::engineio::transports::Transport;
use crate::error::{Error, Result};
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use websocket::{
    client::sync::Client as WsClient,
    dataframe::Opcode,
    sync::stream::{TcpStream, TlsStream},
    ws::dataframe::DataFrame,
    ClientBuilder as WsClientBuilder, Message,
};

pub(super) struct WebsocketSecureTransport {
    client: Arc<Mutex<WsClient<TlsStream<TcpStream>>>>,
}

impl WebsocketSecureTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(address: String, tls_config: Option<TlsConnector>) -> Self {
        let client = WsClientBuilder::new(address[..].as_ref())
            .unwrap()
            .connect_secure(tls_config)
            .unwrap();

        client.set_nonblocking(false).unwrap();

        WebsocketSecureTransport {
            client: Arc::new(Mutex::new(client)),
        }
    }

    pub(super) fn probe(&self) -> Result<()> {
        let mut client = self.client.lock()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        client.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Ping, Bytes::from("probe"))
                .encode_packet()
                .as_ref(),
        )))?;

        // expect to receive a probe packet
        let message = client.recv_message()?;
        if message.take_payload()
            != Packet::new(PacketId::Pong, Bytes::from("probe")).encode_packet()
        {
            return Err(Error::HandshakeError("Error".to_owned()));
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        client.send_message(&Message::binary(Cow::Borrowed(
            Packet::new(PacketId::Upgrade, Bytes::from(""))
                .encode_packet()
                .as_ref(),
        )))?;

        Ok(())
    }
}

impl Transport for WebsocketSecureTransport {
    fn emit(&self, _: String, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut writer = self.client.lock()?;

        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };
        writer.send_message(&message)?;

        Ok(())
    }

    fn poll(&mut self, _: String) -> Result<Bytes> {
        let mut receiver = self.client.lock().unwrap();

        // if this is a binary payload, we mark it as a message
        let received_df = receiver.recv_dataframe().unwrap();
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

    fn name(&self) -> Result<String> {
        Ok("websocket".to_owned())
    }
}
