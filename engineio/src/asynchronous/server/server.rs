use super::accept::WebsocketAcceptor;
use super::accept::{peek_request_type, PollingAcceptor, RequestType, SidGenerator};
use crate::asynchronous::async_socket::Socket;
use crate::asynchronous::async_transports::WebsocketTransport;
use crate::asynchronous::callback::OptionalCallback;
use crate::asynchronous::transport::AsyncTransportType;
use crate::error::Result;
use crate::packet::HandshakePacket;
use crate::Packet;
use bytes::Bytes;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::RwLock;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Sid = String;

#[derive(Clone)]
pub struct ServerOption {
    pub ping_timeout: u64,
    pub ping_interval: u64,
}

impl Default for ServerOption {
    fn default() -> Self {
        Self {
            ping_interval: 20000,
            ping_timeout: 25000,
        }
    }
}

#[derive(Default, Clone)]
pub struct Server {
    pub(crate) inner: Arc<Inner>,
}

#[allow(dead_code)]
pub(crate) struct Inner {
    pub(crate) port: u16,
    pub(crate) id_generator: SidGenerator,
    pub(crate) server_option: ServerOption,
    pub(crate) sockets: RwLock<HashMap<String, Socket>>,

    pub(crate) on_error: OptionalCallback<String>,
    pub(crate) on_open: OptionalCallback<()>,
    pub(crate) on_close: OptionalCallback<()>,
    pub(crate) on_data: OptionalCallback<Bytes>,
    pub(crate) on_packet: OptionalCallback<Packet>,
}

impl Server {
    pub async fn serve(&self) {
        let addr = format!("0.0.0.0:{}", self.inner.port);
        let listener = TcpListener::bind(&addr)
            .await
            .expect("engine-io server can not listen port");

        while let Ok((stream, _)) = listener.accept().await {
            let server = self.clone();
            tokio::spawn(async move { accept_connection(server, stream).await });
        }
    }

    pub async fn emit(&self, sid: &str, packet: Packet) -> Result<()> {
        let sockets = self.inner.sockets.read().await;
        let socket = sockets.get(sid);
        if let Some(socket) = socket {
            socket.emit(packet).await?;
        }
        Ok(())
    }

    pub async fn store_stream(
        &self,
        sid: String,
        ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        let (sender, receiver) = ws_stream.split();
        let transport: AsyncTransportType =
            AsyncTransportType::Websocket(WebsocketTransport::new_for_server(sender, receiver));
        let handshake = self.handshake_packet(vec!["webscocket".to_owned()], Some(sid.clone()));
        let mut socket = Socket::new(
            transport,
            handshake,
            self.inner.on_close.clone(),
            self.inner.on_data.clone(),
            self.inner.on_error.clone(),
            self.inner.on_open.clone(),
            self.inner.on_packet.clone(),
        );
        socket.set_server();
        socket.connect().await?;
        let mut sockets = self.inner.sockets.write().await;
        let _ = sockets.insert(sid, socket);
        Ok(())
    }

    pub fn handshake_packet(&self, upgrades: Vec<String>, sid: Option<Sid>) -> HandshakePacket {
        let sid = match sid {
            Some(sid) => sid,
            None => self.inner.id_generator.generate(),
        };
        HandshakePacket {
            sid,
            ping_timeout: self.inner.server_option.ping_timeout,
            ping_interval: self.inner.server_option.ping_interval,
            upgrades,
        }
    }

    pub fn server_option(&self) -> ServerOption {
        self.inner.server_option.clone()
    }

    pub fn sid(&self) -> Sid {
        self.inner.id_generator.generate()
    }
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            port: 4205,
            id_generator: SidGenerator::default(),
            server_option: ServerOption::default(),
            sockets: Default::default(),

            on_error: OptionalCallback::default(),
            on_open: OptionalCallback::default(),
            on_close: OptionalCallback::default(),
            on_data: OptionalCallback::default(),
            on_packet: OptionalCallback::default(),
        }
    }
}

async fn accept_connection(server: Server, stream: TcpStream) -> Result<()> {
    // TODO: tls
    match peek_request_type(&stream).await {
        Some(RequestType::WsUpgrade(sid)) => {
            WebsocketAcceptor::accept(server, sid, MaybeTlsStream::Plain(stream)).await
        }
        // TODO: polling transport
        _ => PollingAcceptor::accept(server, stream).await,
    }
}

#[cfg(test)]
mod test {

    use crate::{
        asynchronous::{server::builder::ServerBuilder, Client, ClientBuilder},
        PacketId,
    };

    use super::*;

    // #[tokio::test]
    // async fn start_server_for_node() {
    //     let url = crate::test::engine_io_server().unwrap();
    //     let port = url.port().unwrap();
    //     let server = ServerBuilder::new(port).build();
    //     server.serve().await;
    // }

    #[tokio::test]
    async fn test_connection() -> Result<()> {
        start_server();

        let url = crate::test::rust_engine_io_server()?;
        let socket = ClientBuilder::new(url.clone()).build().await?;
        test_connect(socket).await?;

        let socket = ClientBuilder::new(url.clone()).build_websocket().await?;
        test_connect(socket).await?;

        let socket = ClientBuilder::new(url)
            .build_websocket_with_upgrade()
            .await?;
        test_connect(socket).await?;

        Ok(())
    }

    fn start_server() {
        let url = crate::test::rust_engine_io_server().unwrap();
        let port = url.port().unwrap();
        let server = ServerBuilder::new(port).build();
        tokio::spawn(async move {
            server.serve().await;
        });
    }

    async fn test_connect(mut socket: Client) -> Result<()> {
        socket.connect().await?;

        let p = socket.next().await.unwrap()?;
        // Ping
        assert!(matches!(
            p,
            Packet {
                packet_id: PacketId::Ping,
                ..
            }
        ));

        socket.disconnect().await?;

        assert!(!socket.is_connected()?);
        Ok(())
    }
}
