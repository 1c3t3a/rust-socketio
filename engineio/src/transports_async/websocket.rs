use crate::{
    error::Result,
    transport::{AsyncTransport},
    Error, Packet, PacketId,
};
use async_trait::async_trait;
use bytes::{BufMut, Bytes, BytesMut};
use futures_util::{
    stream::{SplitSink, SplitStream, StreamExt},
    SinkExt,
};
use std::{borrow::Cow, str::from_utf8, sync::Arc};
use tokio::{
    net::TcpStream,
    sync::{Mutex, RwLock},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use url::Url;

pub(crate) struct AsyncWebsocketTransport {
    sender: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
    receiver: Arc<Mutex<SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>>>,
    base_url: Arc<RwLock<url::Url>>,
}

impl AsyncWebsocketTransport {
    pub async fn new(base_url: url::Url) -> Result<Self> {
        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "websocket");
        url.set_scheme("ws").unwrap();

        let (ws_stream, _) = connect_async(url.clone()).await?;
        let (sen, rec) = ws_stream.split();

        Ok(AsyncWebsocketTransport {
            receiver: Arc::new(Mutex::new(rec)),
            sender: Arc::new(Mutex::new(sen)),
            base_url: Arc::new(RwLock::new(url)),
        })
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

        let msg = receiver.next().await.ok_or(Error::IllegalWebsocketUpgrade())??;

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
}

#[async_trait]
impl AsyncTransport for AsyncWebsocketTransport {
    async fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let mut sender = self.sender.lock().await;

        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };

        sender.send(message).await?;

        Ok(())
    }

    async fn poll(&self) -> Result<Bytes> {
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

    async fn base_url(&self) -> Result<Url> {
        Ok(self.base_url.read().await.clone())
    }

    async fn set_base_url(&self, base_url: Url) -> Result<()> {
        let mut url = base_url;
        if !url
            .query_pairs()
            .any(|(k, v)| k == "transport" && v == "websocket")
        {
            url.query_pairs_mut().append_pair("transport", "websocket");
        }
        url.set_scheme("ws").unwrap();
        *self.base_url.write().await = url;
        Ok(())
    }
}
