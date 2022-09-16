use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use crate::{
    asynchronous::{
        callback::Callback, server::client::Client as ServerClient, socket::Socket, NameSpace,
    },
    error::Result,
    packet::{Packet, PacketId},
    Error, Event, Payload,
};
use futures_util::StreamExt;
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
                    let mut room = HashMap::new();
                    room.insert(room_name, room_clients);
                    clients.insert(nsp.to_owned(), room);
                }
                Some(rooms) => {
                    if let Some(room_clients) = rooms.get_mut(&room_name) {
                        let _ = room_clients.insert(sid.clone(), client.clone());
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

                let _ = client
                    .emit(Event::Connect, json!({ "sid": sid.clone() }))
                    .await;

                let mut clients = self.clients.write().await;
                let mut ns_clients = HashMap::new();
                ns_clients.insert(nsp, client);
                clients.insert(sid, ns_clients);
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
        asynchronous::server::client::Client as ServerClient,
        asynchronous::{server::builder::ServerBuilder, Client, ClientBuilder},
        test::rust_socket_io_server,
        Event, Payload,
    };

    use futures_util::FutureExt;
    use serde_json::json;

    #[tokio::test]
    async fn test_server() {
        setup();

        let is_recv = Arc::new(AtomicBool::default());
        let is_recv_clone = Arc::clone(&is_recv);
        let callback = move |_payload: Payload, _socket: Client| {
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
            .on(Event::Connect, move |_payload, socket| {
                {
                    async move {
                        let _ = socket.emit("echo", json!("data")).await;
                    }
                    .boxed()
                }
            })
            .connect()
            .await;

        assert!(socket.is_ok());

        // wait recv data
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(is_recv.load(Ordering::SeqCst));
    }

    fn setup() {
        let callback = |_payload: Payload, socket: ServerClient| {
            async move {
                socket.join(vec!["room 1"]).await;
                let _ = socket
                    .emit_to(vec!["room 1"], "echo", json!({"got ack": true}))
                    .await;
                socket.leave(vec!["room 1"]).await;
            }
            .boxed()
        };
        let url = rust_socket_io_server();
        let server = ServerBuilder::new(url.port().unwrap())
            .on("/admin", "echo", callback)
            .build();
        tokio::spawn(async move { server.serve().await });
    }
}
