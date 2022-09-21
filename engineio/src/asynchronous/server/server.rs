use super::accept::{peek_request_type, PollingAcceptor, RequestType, WebsocketAcceptor};
use crate::asynchronous::async_transports::{PollingTransport, WebsocketTransport};
use crate::asynchronous::transport::AsyncTransportType;
use crate::asynchronous::{async_socket::Socket, Client};
use crate::error::Result;
use crate::packet::HandshakePacket;
use crate::{asynchronous::callback::OptionalCallback, packet::build_polling_payload};
use crate::{Packet, PacketId};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::Url;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex, RwLock,
};
use tokio::time::{interval, Instant};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::channel,
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type Sid = Arc<String>;
pub type PollingHandle = (Sender<Bytes>, Receiver<Bytes>);

#[derive(Clone, Debug)]
pub struct ServerOption {
    pub ping_timeout: u64,
    pub ping_interval: u64,
    pub max_payload: usize,
}

impl Default for ServerOption {
    // values copied from node version of engine.io
    fn default() -> Self {
        Self {
            ping_interval: 25000,
            ping_timeout: 20000,
            max_payload: 1024,
        }
    }
}

#[derive(Default)]
pub(crate) struct SidGenerator {
    seq: AtomicUsize,
}

impl SidGenerator {
    pub fn generate(&self) -> Sid {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        Arc::new(base64::encode(format!("{}", seq)))
    }
}

#[derive(Default, Clone)]
pub struct Server {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) port: u16,
    pub(crate) id_generator: SidGenerator,
    pub(crate) server_option: ServerOption,
    pub(crate) clients: RwLock<HashMap<Sid, Client>>,
    pub(crate) polling_handles: Mutex<HashMap<Sid, PollingHandle>>,

    pub(crate) on_open: OptionalCallback<Sid>,
    pub(crate) on_close: OptionalCallback<Sid>,
    pub(crate) on_data: OptionalCallback<(Sid, Bytes)>,
    pub(crate) on_error: OptionalCallback<(Sid, String)>,
    pub(crate) on_packet: OptionalCallback<(Sid, Packet)>,
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

    pub async fn emit(&self, sid: &Sid, packet: Packet) -> Result<()> {
        let clients = self.inner.clients.read().await;
        let client = clients.get(sid);
        if let Some(client) = client {
            client.emit(packet).await?;
        }
        Ok(())
    }

    pub async fn is_connected(&self, sid: &Sid) -> bool {
        let clients = self.inner.clients.read().await;
        match clients.get(sid) {
            Some(s) => s.is_connected(),
            None => false,
        }
    }

    pub async fn store_polling(&self, sid: Sid, peer_addr: &SocketAddr) -> Result<()> {
        let (send_tx, send_rx) = channel(100);
        let (recv_tx, recv_rx) = channel(100);
        let url = Url::parse(&format!("http://{}", peer_addr)).unwrap();
        let transport = PollingTransport::server_new(url, send_tx, recv_rx);

        let mut polling_handles = self.inner.polling_handles.lock().await;
        polling_handles.insert(sid.clone(), (recv_tx, send_rx));
        drop(polling_handles);

        // SAFETY: url is valid to parse
        let transport: AsyncTransportType = AsyncTransportType::Polling(transport);

        let handshake = self.handshake_packet(vec!["webscocket".to_owned()], Some(sid.clone()));
        let mut socket = Socket::new(
            transport,
            handshake,
            false, // server no need to pong
            self.on_close(&sid),
            self.inner.on_data.clone(),
            self.inner.on_error.clone(),
            self.inner.on_open.clone(),
            self.inner.on_packet.clone(),
        );
        socket.set_server();

        let client = Client::new(socket);
        let mut clients = self.inner.clients.write().await;
        let _ = clients.insert(sid.clone(), client.clone());

        // socket.io server will receive Sid by on_open callback,
        // then fetch engine.io client
        client.connect().await?;

        Ok(())
    }

    pub async fn store_stream(
        &self,
        sid: Sid,
        peer_addr: &SocketAddr,
        ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    ) -> Result<()> {
        let (sender, receiver) = ws_stream.split();
        // SAFETY: url is valid to parse
        let url = Url::parse(&format!("http://{}", peer_addr)).unwrap();
        let transport: AsyncTransportType = AsyncTransportType::Websocket(
            WebsocketTransport::new_for_server(sender, receiver, url),
        );
        let handshake = self.handshake_packet(vec!["webscocket".to_owned()], Some(sid.clone()));
        let mut socket = Socket::new(
            transport,
            handshake,
            false, // server no need to pong
            self.on_close(&sid),
            self.inner.on_data.clone(),
            self.inner.on_error.clone(),
            self.inner.on_open.clone(),
            self.inner.on_packet.clone(),
        );
        socket.set_server();

        let client = Client::new(socket);
        let mut clients = self.inner.clients.write().await;
        let _ = clients.insert(sid.clone(), client.clone());

        // socket.io server will receive Sid by on_open callback,
        // then fetch engine.io client
        client.connect().await?;
        self.start_ping_pong(&sid);
        Ok(())
    }

    pub(crate) async fn polling_post(&self, sid: &Sid, data: Bytes) {
        let mut handles = self.inner.polling_handles.lock().await;

        if let Some((ref mut tx, _)) = handles.get_mut(sid) {
            let _ = tx.send(data).await;
        }
    }

    pub(crate) async fn polling_get(&self, sid: &Sid) -> Option<String> {
        let mut handles = self.inner.polling_handles.lock().await;
        if let Some((_, rx)) = handles.get_mut(sid) {
            let mut byte_vec = VecDeque::new();
            while let Ok(bytes) = rx.try_recv() {
                byte_vec.push_back(bytes);
            }
            match build_polling_payload(byte_vec) {
                Some(payload) => return Some(payload),
                None => {
                    let clients = self.inner.clients.read().await;
                    if clients.get(sid).is_some() {
                        return Some(PacketId::Noop.into());
                    }
                }
            };
        }
        None
    }

    pub(crate) async fn close_polling(&self, sid: &Sid) {
        let mut handles = self.inner.polling_handles.lock().await;
        handles.remove(sid);
    }

    pub async fn client(&self, sid: &Sid) -> Option<Client> {
        let clients = self.inner.clients.read().await;
        clients.get(sid).map(|x| x.to_owned())
    }

    pub async fn close_socket(&self, sid: &Sid) {
        let mut sockets = self.inner.clients.write().await;
        if let Some(socket) = sockets.remove(sid) {
            // socket.disconnect will call on_close, on_close will call server.drop_socket,
            // inner.sockets write lock will conflict
            drop(sockets);
            let _ = socket.disconnect().await;
        }
    }

    async fn drop_socket(&self, sid: &Sid) {
        let mut sockets = self.inner.clients.write().await;
        let _ = sockets.remove(sid);
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

    pub fn start_ping_pong(&self, sid: &Sid) {
        let sid = sid.to_owned();
        let server = self.clone();
        let option = server.server_option();
        let timeout = Duration::from_millis(option.ping_timeout + option.ping_interval);
        let mut interval = interval(Duration::from_millis(option.ping_interval));

        tokio::spawn(async move {
            while server.is_connected(&sid).await {
                let ping_packet = Packet {
                    packet_id: PacketId::Ping,
                    data: Bytes::new(),
                };
                if server.emit(&sid, ping_packet).await.is_err() {
                    break;
                };
                let last_pong = server.last_pong(&sid).await;
                match last_pong {
                    Some(instant) if instant.elapsed() < timeout => {}
                    _ => break,
                }
                interval.tick().await;
            }
            server.close_socket(&sid).await;
        });
    }

    pub fn server_option(&self) -> ServerOption {
        self.inner.server_option.clone()
    }

    pub fn generate_sid(&self) -> Sid {
        self.inner.id_generator.generate()
    }

    pub(crate) fn max_payload(&self) -> usize {
        self.inner.server_option.max_payload
    }

    pub(crate) async fn last_pong(&self, sid: &Sid) -> Option<Instant> {
        let sockets = self.inner.clients.read().await;
        Some(sockets.get(sid)?.last_pong().await)
    }

    fn on_close(&self, sid: &Sid) -> OptionalCallback<Sid> {
        let sid = sid.to_owned();
        let on_close = self.inner.on_close.clone();
        let server = self.clone();

        OptionalCallback::new(move |p| {
            let sid = sid.clone();
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
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            port: 80,
            id_generator: SidGenerator::default(),
            server_option: ServerOption::default(),
            clients: Default::default(),
            polling_handles: Default::default(),

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
    match peek_request_type(&stream, &peer_addr, server.max_payload()).await {
        Some(RequestType::WsUpgrade(sid)) => {
            WebsocketAcceptor::accept(server, sid, MaybeTlsStream::Plain(stream), &peer_addr).await
        }
        _ => PollingAcceptor::accept(server, stream, &peer_addr).await,
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::{
        asynchronous::{server::builder::ServerBuilder, Client, ClientBuilder},
        PacketId,
    };
    use tokio::sync::mpsc::Receiver;

    #[tokio::test]
    async fn test_connection() -> Result<()> {
        let url = crate::test::rust_engine_io_server().unwrap();
        let (mut rx, server) = start_server(url.clone());

        let socket = ClientBuilder::new(url.clone()).build().await?;
        test_data_transport(server.clone(), socket, &mut rx).await?;

        let socket = ClientBuilder::new(url.clone()).build_websocket().await?;
        test_data_transport(server.clone(), socket, &mut rx).await?;

        let socket = ClientBuilder::new(url)
            .build_websocket_with_upgrade()
            .await?;
        test_data_transport(server, socket, &mut rx).await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_pong_timeout() -> Result<()> {
        let url = crate::test::rust_engine_io_timeout_server().unwrap();
        let _ = start_server(url.clone());

        let socket = ClientBuilder::new(url.clone())
            .should_pong_for_test(false)
            .build()
            .await?;
        test_transport_timeout(socket).await?;

        let socket = ClientBuilder::new(url.clone())
            .should_pong_for_test(false)
            .build_websocket()
            .await?;
        test_transport_timeout(socket).await?;

        let socket = ClientBuilder::new(url)
            .should_pong_for_test(false)
            .build_websocket_with_upgrade()
            .await?;
        test_transport_timeout(socket).await?;

        Ok(())
    }

    fn start_server(url: Url) -> (Receiver<String>, Server) {
        let port = url.port().unwrap();
        let server_option = ServerOption {
            ping_timeout: 50,
            ping_interval: 50,
            max_payload: 1024,
        };
        let (builder, rx) = setup(port, server_option);
        let server = builder.build();
        let server_clone = server.clone();

        tokio::spawn(async move {
            server_clone.serve().await;
        });

        (rx, server)
    }

    fn poll_client(mut client: Client) {
        tokio::spawn(async move { while client.next().await.is_some() {} });
    }

    async fn test_data_transport(
        server: Server,
        mut client: Client,
        server_rx: &mut Receiver<String>,
    ) -> Result<()> {
        client.connect().await?;

        let mut client_clone = client.clone();
        tokio::spawn(async move { while client_clone.next().await.is_some() {} });

        // Ping
        assert!(matches!(
            client.next().await.unwrap()?,
            Packet {
                packet_id: PacketId::Ping,
                ..
            }
        ));

        client
            .emit(Packet::new(PacketId::Message, Bytes::from("msg")))
            .await?;

        let mut sid = Arc::new("".to_owned());

        // ignore item send by last client
        while let Some(item) = server_rx.recv().await {
            if item.starts_with("open") {
                let items: Vec<&str> = item.split(' ').collect();
                sid = Arc::new(items[1].to_owned());
                break;
            }
        }

        let client = server.client(&sid).await;
        assert!(client.is_some());
        let client = client.unwrap();
        poll_client(client.clone());

        // wait ping pong
        tokio::time::sleep(Duration::from_millis(100)).await;

        client.disconnect().await?;

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
        assert!(!client.is_connected());

        Ok(())
    }

    async fn test_transport_timeout(mut client: Client) -> Result<()> {
        client.connect().await?;

        let client_clone = client.clone();
        tokio::spawn(async move {
            loop {
                let next = client.next().await;
                if next.is_none() {
                    break;
                }
            }
        });

        tokio::time::sleep(Duration::from_millis(200)).await;

        // closed by server
        assert!(!client_clone.is_connected());

        Ok(())
    }

    fn setup(port: u16, server_option: ServerOption) -> (ServerBuilder, Receiver<String>) {
        let (tx, rx) = tokio::sync::mpsc::channel(100);
        let tx = Arc::new(tx);
        let tx1 = Arc::clone(&tx);
        let tx2 = Arc::clone(&tx);
        let tx3 = Arc::clone(&tx);
        let tx4 = Arc::clone(&tx);
        (
            ServerBuilder::new(port)
                .server_option(server_option)
                .on_open(move |sid| {
                    let tx = Arc::clone(&tx1);
                    Box::pin(async move {
                        let _ = tx.send(format!("open {}", sid)).await;
                    })
                })
                .on_packet(move |(_sid, packet)| {
                    let tx = Arc::clone(&tx2);
                    Box::pin(async move {
                        let _ = tx.send(String::from(packet.packet_id)).await;
                    })
                })
                .on_data(move |(_sid, data)| {
                    let tx = Arc::clone(&tx3);
                    Box::pin(async move {
                        let data = std::str::from_utf8(&data).unwrap();
                        let _ = tx.send(data.to_owned()).await;
                    })
                })
                .on_close(move |_sid| {
                    let tx = Arc::clone(&tx4);
                    Box::pin(async move {
                        let _ = tx.send("close".to_owned()).await;
                    })
                })
                .on_error(|(_sid, error)| {
                    Box::pin(async move {
                        println!("Error {}", error);
                    })
                }),
            rx,
        )
    }
}
