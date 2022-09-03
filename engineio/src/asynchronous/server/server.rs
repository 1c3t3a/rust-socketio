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
use std::sync::Arc;
use std::{collections::HashMap, net::SocketAddr};
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

        while let Ok((stream, peer_addr)) = listener.accept().await {
            let server = self.clone();
            tokio::spawn(async move { accept_connection(server, stream, peer_addr).await });
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

    pub async fn is_connected(&self, sid: &str) -> Result<bool> {
        let sockets = self.inner.sockets.read().await;
        match sockets.get(sid) {
            Some(s) => s.is_connected(),
            None => Ok(false),
        }
    }

    pub async fn store_stream(
        &self,
        sid: String,
        peer_addr: &SocketAddr,
        ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        let (sender, receiver) = ws_stream.split();
        let url = format!("http://{}", peer_addr);
        let transport: AsyncTransportType = AsyncTransportType::Websocket(
            WebsocketTransport::new_for_server(sender, receiver, url),
        );
        let handshake = self.handshake_packet(vec!["webscocket".to_owned()], Some(sid.clone()));
        let mut socket = Socket::new(
            transport,
            handshake,
            self.on_close(&sid),
            self.inner.on_data.clone(),
            self.inner.on_error.clone(),
            self.inner.on_open.clone(),
            self.inner.on_packet.clone(),
        );

        socket.set_server();
        socket.connect().await?;
        poll_packet(socket.clone());

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

    fn on_close(&self, sid: &str) -> OptionalCallback<()> {
        let sid_clone = sid.to_owned();
        let on_close = self.inner.on_close.clone();
        let server = self.clone();

        OptionalCallback::new(move |p| {
            let sid = sid_clone.clone();
            let on_close = on_close.clone();
            let server = server.clone();
            Box::pin(async move {
                if let Some(on_close) = on_close.as_deref() {
                    on_close(p).await;
                }
                server.drop_socket(&sid).await;
            })
        })
    }

    async fn drop_socket(&self, sid: &str) {
        let mut sockets = self.inner.sockets.write().await;
        let _ = sockets.remove(sid);
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

async fn accept_connection(server: Server, stream: TcpStream, peer_addr: SocketAddr) -> Result<()> {
    // TODO: tls
    match peek_request_type(&stream, &peer_addr).await {
        Some(RequestType::WsUpgrade(sid)) => {
            WebsocketAcceptor::accept(server, sid, MaybeTlsStream::Plain(stream), &peer_addr).await
        }
        // TODO: polling transport
        _ => PollingAcceptor::accept(server, stream, &peer_addr).await,
    }
}

fn poll_packet(mut socket: Socket) {
    tokio::spawn(async move {
        while let Some(packet) = socket.next().await {
            let result = match packet {
                Ok(p) => socket.handle_inconming_packet(p).await,
                Err(e) => Err(e),
            };
            if result.is_err() {
                // TODO: handle error
                break;
            }
        }
    });
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::{
        asynchronous::{server::builder::ServerBuilder, Client, ClientBuilder},
        PacketId,
    };
    use tokio::sync::{mpsc::Receiver, Mutex};

    #[tokio::test]
    async fn test_connection() -> Result<()> {
        let mut rx = start_server();

        let url = crate::test::rust_engine_io_server()?;
        let socket = ClientBuilder::new(url.clone()).build().await?;
        test_data_transport(socket, &mut rx).await?;

        let socket = ClientBuilder::new(url.clone()).build_websocket().await?;
        test_data_transport(socket, &mut rx).await?;

        let socket = ClientBuilder::new(url)
            .build_websocket_with_upgrade()
            .await?;
        test_data_transport(socket, &mut rx).await?;

        Ok(())
    }

    fn start_server() -> Receiver<String> {
        let url = crate::test::rust_engine_io_server().unwrap();
        let port = url.port().unwrap();
        let (builder, rx) = setup(port);
        let server = builder.build();

        tokio::spawn(async move {
            server.serve().await;
        });

        rx
    }

    async fn test_data_transport(
        mut socket: Client,
        server_rx: &mut Receiver<String>,
    ) -> Result<()> {
        socket.connect().await?;

        // Ping
        assert!(matches!(
            socket.next().await.unwrap()?,
            Packet {
                packet_id: PacketId::Ping,
                ..
            }
        ));

        socket
            .emit(Packet::new(PacketId::Message, Bytes::from("msg")))
            .await?;

        socket.disconnect().await?;

        let mut receive_pong = false;
        let mut receive_msg = false;

        while let Some(item) = server_rx.recv().await {
            match item.as_str() {
                "3" => receive_pong = true,
                "msg" => receive_msg = true,
                "close" => break,
                _ => {}
            }
        }

        assert!(receive_pong);
        assert!(receive_msg);
        assert!(!socket.is_connected()?);

        Ok(())
    }

    fn setup(port: u16) -> (ServerBuilder, Receiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let tx = Arc::new(Mutex::new(tx));
        let tx1 = Arc::clone(&tx);
        let tx2 = Arc::clone(&tx);
        let tx3 = Arc::clone(&tx);
        let tx4 = Arc::clone(&tx);
        (
            ServerBuilder::new(port)
                .on_open(move |_| {
                    let tx = Arc::clone(&tx1);
                    Box::pin(async move {
                        let guard = tx.lock().await;
                        let _ = guard.send("open".to_owned()).await;
                    })
                })
                .on_packet(move |packet| {
                    let tx = Arc::clone(&tx2);
                    Box::pin(async move {
                        let guard = tx.lock().await;
                        let _ = guard.send(String::from(packet.packet_id)).await;
                    })
                })
                .on_data(move |data| {
                    let tx = Arc::clone(&tx3);
                    Box::pin(async move {
                        let data = std::str::from_utf8(&data).unwrap();
                        let guard = tx.lock().await;
                        let _ = guard.send(data.to_owned()).await;
                    })
                })
                .on_close(move |_| {
                    let tx = Arc::clone(&tx4);
                    Box::pin(async move {
                        let guard = tx.lock().await;
                        let _ = guard.send("close".to_owned()).await;
                    })
                })
                .on_error(|error| {
                    Box::pin(async move {
                        println!("Error {}", error);
                    })
                }),
            rx,
        )
    }
}
