use crate::engineio::packet::{Packet, PacketId};
use crate::engineio::transports::Transport;
use crate::error::{Error, Result};
use bytes::{BufMut, Bytes, BytesMut};
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use websocket::{
    dataframe::Opcode, receiver::Reader, sync::stream::TcpStream, sync::Writer,
    ws::dataframe::DataFrame, ClientBuilder as WsClientBuilder, Message,
};

pub(super) struct WebsocketTransport {
    sender: Arc<Mutex<Writer<TcpStream>>>,
    receiver: Arc<Mutex<Reader<TcpStream>>>,
}

impl WebsocketTransport {
    /// Creates an instance of `TransportClient`.
    pub fn new(address: String) -> Self {
        let client = WsClientBuilder::new(address[..].as_ref())
            .unwrap()
            .connect_insecure()
            .unwrap();

        client.set_nonblocking(false).unwrap();

        let (receiver, sender) = client.split().unwrap();

        WebsocketTransport {
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    pub(super) fn probe(&self) -> Result<()> {
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
            return Err(Error::HandshakeError("Error".to_owned()));
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
    fn emit(&self, _: String, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut sender = self.sender.lock()?;

        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };
        sender.send_message(&message)?;

        Ok(())
    }

    fn poll(&mut self, _: String) -> Result<Bytes> {
        let mut receiver = self.receiver.lock().unwrap();

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
