use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    asynchronous::{
        ack::AckId, callback::Callback, server::client::Client as ServerClient, socket::Socket,
        NameSpace,
    },
    error::Result,
    packet::{Packet, PacketId},
    Error, Event, Payload,
};
use futures_util::{future::BoxFuture, StreamExt};
use log::trace;
use rust_engineio::{
    asynchronous::{server::Server as EngineServer, Sid},
    PacketId as EnginePacketId,
};
use serde_json::json;
use tokio::sync::{mpsc::Receiver, RwLock};

type Room = String;
type Rooms = HashMap<NameSpace, HashMap<Room, HashMap<Sid, ServerClient>>>;
type On = HashMap<Event, Callback<ServerClient>>;

pub struct Server {
    pub(crate) on: HashMap<NameSpace, Arc<RwLock<On>>>,
    pub(crate) rooms: RwLock<Rooms>,
    pub(crate) clients: RwLock<HashMap<Sid, HashMap<NameSpace, ServerClient>>>,
    pub(crate) engine_server: EngineServer,
    pub(crate) sid_generator: SidGenerator,
}

impl Server {
    #[allow(dead_code)]
    pub async fn serve(self: Arc<Self>) {
        self.engine_server.serve().await
    }

    pub(crate) fn recv_event(self: &Arc<Self>, mut event_rx: Receiver<(Sid, EnginePacketId)>) {
        let server = self.to_owned();
        tokio::spawn(async move {
            while let Some((sid, event)) = event_rx.recv().await {
                match event {
                    EnginePacketId::Open => server.create_client(sid).await,
                    EnginePacketId::Close => server.drop_client(&sid).await,
                    _ => {}
                }
            }
        });
    }

    pub(crate) async fn emit_to<E, D>(
        self: &Arc<Self>,
        nsp: &str,
        rooms: Vec<&str>,
        event: E,
        data: D,
    ) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        let clients = self.rooms.read().await;
        let event = event.into();
        let payload = data.into();
        if let Some(room_clients) = clients.get(nsp) {
            for room_name in rooms {
                if let Some(room) = room_clients.get(room_name) {
                    for client in room.values() {
                        client.emit(event.clone(), payload.clone()).await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn emit_to_with_ack<F, E, D>(
        &self,
        nsp: &str,
        rooms: Vec<&str>,
        event: E,
        data: D,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: for<'a> std::ops::FnMut(Payload, ServerClient, Option<AckId>) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync
            + Clone,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let clients = self.rooms.read().await;
        let event = event.into();
        let payload = data.into();
        if let Some(room_clients) = clients.get(nsp) {
            for room_name in rooms {
                if let Some(room) = room_clients.get(room_name) {
                    for client in room.values() {
                        client
                            .emit_with_ack(
                                event.clone(),
                                payload.clone(),
                                timeout,
                                callback.clone(),
                            )
                            .await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn join<T: Into<String>>(
        self: &Arc<Self>,
        nsp: &str,
        rooms: Vec<T>,
        sid: Sid,
        client: ServerClient,
    ) {
        let mut clients = self.rooms.write().await;
        for room_name in rooms {
            let room_name = room_name.into();
            match clients.get_mut(nsp) {
                None => {
                    let mut room_clients = HashMap::new();
                    room_clients.insert(sid.clone(), client.clone());
                    let mut rooms = HashMap::new();
                    rooms.insert(room_name, room_clients);
                    clients.insert(nsp.to_owned(), rooms);
                }
                Some(rooms) => {
                    if let Some(room_clients) = rooms.get_mut(&room_name) {
                        let _ = room_clients.insert(sid.clone(), client.clone());
                    } else {
                        let mut room_clients = HashMap::new();
                        room_clients.insert(sid.clone(), client.clone());
                        rooms.insert(room_name, room_clients);
                    }
                }
            };
        }
    }

    pub(crate) async fn leave(self: &Arc<Self>, nsp: &str, rooms: Vec<&str>, sid: Sid) {
        let mut clients = self.rooms.write().await;
        for room_name in rooms {
            if let Some(room_clients) = clients.get_mut(nsp) {
                if let Some(clients) = room_clients.get_mut(room_name) {
                    clients.remove(&sid);
                }
            };
        }
    }

    async fn create_client(self: &Arc<Self>, sid: Sid) {
        if let Some(engine_client) = self.engine_server.client(&sid).await {
            let mut socket = Socket::new_server(engine_client);
            let next = socket.next().await;
            // TODO: support multiple namespace
            // first packet should be Connect
            if let Some(Ok(packet)) = next {
                self.handle_engine_packet(packet, sid.clone(), socket.clone())
                    .await;
            }
        }
    }

    async fn handle_engine_packet(
        self: &Arc<Self>,
        packet: Packet,
        engine_sid: Sid,
        socket: Socket,
    ) {
        if packet.packet_type == PacketId::Connect {
            let sid = self.sid_generator.generate(engine_sid);
            let nsp = packet.nsp.clone();
            if let Some(on) = self.on.get(&nsp) {
                let client = ServerClient::new(
                    socket,
                    nsp.clone(),
                    sid.clone(),
                    on.to_owned(),
                    self.clone(),
                );

                poll_client(client.clone());

                if client
                    .handshake(json!({ "sid": sid.clone() }).to_string())
                    .await
                    .is_ok()
                {
                    let mut clients = self.clients.write().await;
                    let mut ns_clients = HashMap::new();
                    ns_clients.insert(nsp, client);
                    clients.insert(sid, ns_clients);
                }
            }
        }
    }

    async fn drop_client(&self, sid: &Sid) {
        let mut clients = self.clients.write().await;
        let _ = clients.remove(sid);
    }
}

#[derive(Default)]
pub(crate) struct SidGenerator {
    seq: AtomicUsize,
}

impl SidGenerator {
    pub fn generate(&self, engine_sid: Sid) -> Sid {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst);
        Arc::new(base64::encode(format!("{}-{}", engine_sid, seq)))
    }
}

fn poll_client(mut client: ServerClient) {
    tokio::runtime::Handle::current().spawn(async move {
        loop {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just logs on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            let next = client.next().await;
            match next {
                Some(e @ Err(Error::IncompleteResponseFromEngineIo(_))) => {
                    trace!("Network error occured: {}", e.unwrap_err());
                }
                None => break,
                _ => {}
            }
        }
    });
}

#[cfg(test)]
mod test {
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };

    use crate::{
        asynchronous::{server::builder::ServerBuilder, Client, ClientBuilder},
        asynchronous::{server::client::Client as ServerClient, AckId},
        test::rust_socket_io_server,
        Event, Payload,
    };

    use futures_util::FutureExt;
    use serde_json::json;

    #[tokio::test]
    async fn test_server() {
        setup();
        test_emit().await;
        test_client_ask_ack().await;
        test_server_ask_ack().await;
    }

    async fn test_emit() {
        let is_recv = Arc::new(AtomicBool::default());
        let is_recv_clone = Arc::clone(&is_recv);

        let callback = move |_: Payload, _: Client, _: Option<AckId>| {
            let is_recv = is_recv_clone.clone();
            async move {
                is_recv.store(true, Ordering::SeqCst);
            }
            .boxed()
        };

        let url = rust_socket_io_server();
        let socket = ClientBuilder::new(url)
            .namespace("/admin")
            .on("echo", callback)
            .on(Event::Connect, move |_payload, socket, _| {
                async move {
                    socket.emit("echo", json!("data")).await.expect("success");
                }
                .boxed()
            })
            .connect()
            .await;

        assert!(socket.is_ok());

        // wait recv data
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(is_recv.load(Ordering::SeqCst));
    }

    async fn test_client_ask_ack() {
        let is_client_ack = Arc::new(AtomicBool::default());
        let is_client_ack_clone = Arc::clone(&is_client_ack);

        let client_ack_callback =
            move |_payload: Payload, _socket: Client, _need_ack: Option<AckId>| {
                let is_client_ack = is_client_ack_clone.clone();
                async move {
                    is_client_ack.store(true, Ordering::SeqCst);
                }
                .boxed()
            };

        let url = rust_socket_io_server();
        let socket = ClientBuilder::new(url)
            .namespace("/admin")
            .on(Event::Connect, move |_payload, socket, _| {
                let client_ack_callback = client_ack_callback.clone();
                async move {
                    socket
                        .emit_with_ack(
                            "client_ack",
                            json!("data"),
                            Duration::from_millis(200),
                            client_ack_callback,
                        )
                        .await
                        .expect("success");
                }
                .boxed()
            })
            .connect()
            .await;

        assert!(socket.is_ok());

        // wait recv data
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(is_client_ack.load(Ordering::SeqCst));
    }

    async fn test_server_ask_ack() {
        let is_server_ask_ack = Arc::new(AtomicBool::default());
        let is_server_recv_ack = Arc::new(AtomicBool::default());
        let is_server_ask_ack_clone = Arc::clone(&is_server_ask_ack);
        let is_server_recv_ack_clone = Arc::clone(&is_server_recv_ack);

        let server_ask_ack = move |_payload: Payload, socket: Client, need_ack: Option<AckId>| {
            let is_server_ask_ack = is_server_ask_ack_clone.clone();
            async move {
                assert!(need_ack.is_some());
                if let Some(ack_id) = need_ack {
                    socket.ack(ack_id, json!("")).await.expect("success");
                    is_server_ask_ack.store(true, Ordering::SeqCst);
                }
            }
            .boxed()
        };

        let server_recv_ack =
            move |_payload: Payload, _socket: Client, _need_ack: Option<AckId>| {
                let is_server_recv_ack = is_server_recv_ack_clone.clone();
                async move {
                    is_server_recv_ack.store(true, Ordering::SeqCst);
                }
                .boxed()
            };

        let url = rust_socket_io_server();
        let socket = ClientBuilder::new(url)
            .namespace("/admin")
            .on("server_ask_ack", server_ask_ack)
            .on("server_recv_ack", server_recv_ack)
            .on(Event::Connect, move |_payload, socket, _| {
                async move {
                    socket
                        .emit("trigger_server_ack", json!("data"))
                        .await
                        .expect("success");
                }
                .boxed()
            })
            .connect()
            .await;

        assert!(socket.is_ok());

        // wait recv data
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(is_server_ask_ack.load(Ordering::SeqCst));
        assert!(is_server_recv_ack.load(Ordering::SeqCst));
    }

    fn setup() {
        let echo_callback =
            move |_payload: Payload, socket: ServerClient, _need_ack: Option<AckId>| {
                async move {
                    socket.join(vec!["room 1"]).await;
                    socket
                        .emit_to(vec!["room 1"], "echo", json!(""))
                        .await
                        .expect("emit success");
                    socket.leave(vec!["room 1"]).await;
                }
                .boxed()
            };

        let client_ack = move |_payload: Payload, socket: ServerClient, need_ack: Option<AckId>| {
            async move {
                if let Some(ack_id) = need_ack {
                    socket
                        .ack(ack_id, json!("ack to client"))
                        .await
                        .expect("success");
                }
            }
            .boxed()
        };

        let server_recv_ack =
            move |_payload: Payload, socket: ServerClient, _need_ack: Option<AckId>| {
                async move {
                    socket
                        .emit("server_recv_ack", json!(""))
                        .await
                        .expect("success");
                }
                .boxed()
            };

        let trigger_ack = move |_message: Payload, socket: ServerClient, _| {
            async move {
                socket.join(vec!["room 2"]).await;
                socket
                    .emit_to_with_ack(
                        vec!["room 2"],
                        "server_ask_ack",
                        json!(true),
                        Duration::from_millis(400),
                        server_recv_ack,
                    )
                    .await
                    .expect("success");
                socket.leave(vec!["room 2"]).await;
            }
            .boxed()
        };

        let url = rust_socket_io_server();
        let server = ServerBuilder::new(url.port().unwrap())
            .on("/admin", "echo", echo_callback)
            .on("/admin", "client_ack", client_ack)
            .on("/admin", "trigger_server_ack", trigger_ack)
            .build();

        tokio::spawn(async move { server.serve().await });
    }
}
