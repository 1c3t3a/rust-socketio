use std::{collections::HashMap, ops::Deref, pin::Pin, sync::Arc, time::Duration};

use futures_util::{future::BoxFuture, Stream, StreamExt};
use rust_engineio::asynchronous::Sid;
use tokio::sync::RwLock;

use crate::{
    asynchronous::{
        ack::AckId, callback::Callback, client::client::CommonClient, server::server::Server,
        socket::Socket,
    },
    error::Result,
    packet::Packet,
    Event, Payload,
};

#[derive(Clone)]
pub struct Client {
    client: CommonClient<Self>,
    server: Arc<Server>,
    sid: Sid,
}

impl Client {
    pub(crate) fn new<T: Into<String>>(
        socket: Socket,
        namespace: T,
        sid: Sid,
        on: Arc<RwLock<HashMap<Event, Callback<Self>>>>,
        server: Arc<Server>,
    ) -> Self {
        let server_clone = server.clone();
        let sid_clone = sid.clone();
        let client = CommonClient::new(
            socket,
            namespace,
            on,
            Arc::new(move |c| Client {
                sid: sid_clone.clone(),
                client: c,
                server: server_clone.clone(),
            }),
        );

        Self {
            sid,
            client,
            server,
        }
    }

    pub async fn join<T: Into<String>>(&self, rooms: Vec<T>) {
        self.server
            .join(&self.client.nsp, rooms, self.sid.clone(), self.clone())
            .await;
    }

    pub async fn leave(&self, rooms: Vec<&str>) {
        self.server.leave(&self.client.nsp, rooms, &self.sid).await;
    }

    pub async fn emit_to<E, D>(&self, rooms: Vec<&str>, event: E, data: D) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        self.server
            .emit_to(&self.client.nsp, rooms, event, data)
            .await
    }

    pub async fn emit_to_with_ack<F, E, D>(
        &self,
        rooms: Vec<&str>,
        event: E,
        data: D,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: for<'a> std::ops::FnMut(Payload, Self, Option<AckId>) -> BoxFuture<'static, ()>
            + 'static
            + Send
            + Sync
            + Clone,
        E: Into<Event>,
        D: Into<Payload>,
    {
        self.server
            .emit_to_with_ack(&self.client.nsp, rooms, event, data, timeout, callback)
            .await
    }
}

impl Deref for Client {
    type Target = CommonClient<Client>;
    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl Stream for Client {
    type Item = Result<Packet>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.client.poll_next_unpin(cx)
    }
}
