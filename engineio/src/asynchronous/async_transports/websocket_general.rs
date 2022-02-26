use std::{borrow::Cow, future::Future, pin::Pin, str::from_utf8, sync::Arc, task::Poll};

use crate::{error::Result, Error, Packet, PacketId};
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{
    ready,
    stream::{SplitSink, SplitStream},
    SinkExt, Stream, StreamExt,
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

/// A convenience type that implements stream and shadows the lifetime
/// of `AsyncWebsocketGeneralTransport`.
pub(crate) struct WsStream<'a> {
    inner: &'a AsyncWebsocketGeneralTransport,
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

    pub(crate) fn stream(&self) -> WsStream<'_> {
        WsStream { inner: self }
    }

    pub(crate) async fn poll(&self) -> Result<Bytes> {
        let mut receiver = self.receiver.lock().await;

        let message = receiver.next().await.ok_or(Error::IncompletePacket())??;
        if message.is_binary() {
            let data = message.into_data();
            let mut msg = BytesMut::with_capacity(data.len() + 1);
            msg.put_u8(PacketId::Message as u8);
            msg.put(data.as_ref());

            Ok(msg.freeze())
        } else {
            Ok(Bytes::from(message.into_data()))
        }
    }
}

impl Stream for WsStream<'_> {
    type Item = Result<Bytes>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match ready!(Pin::new(&mut Box::pin(self.inner.poll())).poll(cx)) {
            Ok(val) => Poll::Ready(Some(Ok(val))),
            Err(err) => Poll::Ready(Some(Err(err))),
        }
    }
}
