use std::{borrow::Cow, str::from_utf8, sync::Arc};

use crate::{error::Result, Error, Packet, PacketId};
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{
    stream::{SplitSink, SplitStream},
    FutureExt, SinkExt, Stream, StreamExt,
};
use tokio::{net::TcpStream, sync::Mutex};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

/// A general purpose asynchronous websocket transport type. Holds
/// the sender and receiver stream of a websocket connection
/// and implements the common methods `update`, `poll` and `emit`.
pub(crate) struct AsyncWebsocketGeneralTransport {
    sender: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    receiver: Arc<Mutex<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
}

impl AsyncWebsocketGeneralTransport {
    pub(crate) async fn new(
        sender: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>,
        receiver: SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>,
    ) -> Self {
        AsyncWebsocketGeneralTransport {
            sender: Arc::new(Mutex::new(sender)),
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) async fn upgrade(&self) -> Result<()> {
        let mut receiver = self.receiver.lock().await;
        let mut sender = self.sender.lock().await;

        sender
            .send(Message::text(Cow::Borrowed(from_utf8(&Bytes::from(
                Packet::new(PacketId::Ping, Bytes::from("probe")),
            ))?)))
            .await?;

        let msg = receiver
            .next()
            .await
            .ok_or(Error::IllegalWebsocketUpgrade())??;

        if msg.into_data() != Bytes::from(Packet::new(PacketId::Pong, Bytes::from("probe"))) {
            return Err(Error::InvalidPacket());
        }

        sender
            .send(Message::text(Cow::Borrowed(from_utf8(&Bytes::from(
                Packet::new(PacketId::Upgrade, Bytes::from("")),
            ))?)))
            .await?;

        Ok(())
    }

    pub(crate) async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut sender = self.sender.lock().await;

        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };

        sender.send(message).await?;

        Ok(())
    }

    pub(crate) fn stream(&self) -> impl Stream<Item = Result<Bytes>> + '_ {
        self.receiver
            .lock()
            .into_stream()
            .then(|mut rec| async move {
                let msg = rec.next().await.ok_or(Error::IncompletePacket())??;
                if msg.is_binary() {
                    let data = msg.into_data();
                    let mut msg = BytesMut::with_capacity(data.len() + 1);
                    msg.put_u8(PacketId::Message as u8);
                    msg.put(data.as_ref());

                    Ok(msg.freeze())
                } else {
                    Ok(Bytes::from(msg.into_data()))
                }
            })
    }
}
