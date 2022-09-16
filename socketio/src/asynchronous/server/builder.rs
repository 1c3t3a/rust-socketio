use crate::asynchronous::server::server::Server;
use crate::asynchronous::{callback::Callback, server::client::Client};
use crate::asynchronous::{AckId, NameSpace};
use crate::{Event, Payload};
use futures_util::future::BoxFuture;
use rust_engineio::{
    asynchronous::server::{ServerBuilder as EngineServerBuilder, ServerOption},
    PacketId,
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[allow(dead_code)]
pub struct ServerBuilder {
    server_option: ServerOption,
    on: HashMap<NameSpace, HashMap<Event, Callback<Client>>>,
    builder: EngineServerBuilder,
}

#[allow(dead_code)]
impl ServerBuilder {
    pub fn new(port: u16) -> Self {
        Self {
            builder: EngineServerBuilder::new(port),
            server_option: Default::default(),
            on: Default::default(),
        }
    }

    pub fn server_option(mut self, server_option: ServerOption) -> Self {
        self.builder = self.builder.server_option(server_option);
        self
    }

    pub fn on<S: Into<String>, T: Into<Event>, F>(
        mut self,
        namespace: S,
        event: T,
        callback: F,
    ) -> Self
    where
        F: for<'a> std::ops::FnMut(Payload, Client, Option<AckId>) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync,
    {
        let namespace = namespace.into();
        if let Some(on) = self.on.get_mut(&namespace) {
            on.insert(event.into(), Callback::new(callback));
        } else {
            let mut on = HashMap::new();
            on.insert(event.into(), Callback::new(callback));
            self.on.insert(namespace, on);
        }
        self
    }

    pub fn build(self) -> Arc<Server> {
        // TODO: channel size can be configured
        let (event_tx, event_rx) = tokio::sync::mpsc::channel(100);
        let tx_clone = event_tx.clone();

        let engine_server = self
            .builder
            .on_open(move |sid| {
                let tx = event_tx.clone();
                Box::pin(async move {
                    let _ = tx.send((sid, PacketId::Open)).await;
                })
            })
            .on_close(move |sid| {
                let tx = tx_clone.clone();
                Box::pin(async move {
                    let _ = tx.send((sid, PacketId::Close)).await;
                })
            })
            .build();

        let mut on = HashMap::new();
        for (k, v) in self.on.into_iter() {
            on.insert(k, Arc::new(RwLock::new(v)));
        }

        let server = Arc::new(Server {
            on,
            engine_server,
            rooms: Default::default(),
            clients: Default::default(),
            sid_generator: Default::default(),
        });

        server.recv_event(event_rx);

        server
    }
}
