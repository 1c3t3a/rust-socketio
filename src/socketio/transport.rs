use crate::engineio::{
    packet::{Error, Packet as EnginePacket, PacketId as EnginePacketId},
    socket::EngineSocket,
};
use crate::socketio::packet::{Packet as SocketPacket, PacketId as SocketPacketId};
use either::*;
use std::sync::{Arc, Mutex};

struct TransportClient {
    engine_socket: Arc<Mutex<EngineSocket>>,
    host: Arc<String>,
}

impl TransportClient {
    pub fn new(address: String) -> Self {
        TransportClient {
            engine_socket: Arc::new(Mutex::new(EngineSocket::new(false))),
            host: Arc::new(address),
        }
    }

    pub async fn connect(&self) -> Result<(), Error> {
        self.engine_socket
            .as_ref()
            .lock()
            .unwrap()
            .bind(self.host.as_ref().to_string())
            .await?;

        let open_packet = SocketPacket::new(SocketPacketId::Connect, String::from("/"), None, None, None);

        dbg!(self.send(open_packet).await)?;
        Ok(())
    }

    pub async fn send(&self, packet: SocketPacket) -> Result<(), Error> {
        let engine_packet = dbg!(EnginePacket::new(EnginePacketId::Message, packet.encode()));

        self.engine_socket
            .lock()
            .unwrap()
            .emit(engine_packet)
            .await
    }

    pub async fn emit(
        &self,
        event: &str,
        data: String,
        namespace: Option<String>,
    ) -> Result<(), Error> {
        let socket_packet = match namespace {
            Some(nsp) => SocketPacket::new(
                SocketPacketId::Event,
                nsp,
                Some(vec![Left(data)]),
                None,
                None,
            ),
            None => SocketPacket::new(
                SocketPacketId::Event,
                String::from("/"),
                Some(vec![Left(data)]),
                None,
                None,
            ),
        };

        self.send(socket_packet).await
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[actix_rt::test]
    async fn it_works() {
        let socket = TransportClient::new(String::from("http://localhost:4200"));

        socket.connect().await.unwrap();

       socket
            .emit(&"test", String::from("[\"test\", \"Du stinkst\"]"), None)
            .await
            .unwrap();
    }
}
