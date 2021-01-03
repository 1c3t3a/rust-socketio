use crate::engineio::{
    packet::{Error, Packet as EnginePacket, PacketId as EnginePacketId},
    socket::EngineSocket,
};
use crate::socketio::packet::{Packet as SocketPacket, PacketId as SocketPacketId};
use either::*;
use std::sync::{Arc, Mutex};

/// A struct that handles communication in the socket.io protocol.
struct TransportClient {
    engine_socket: Arc<Mutex<EngineSocket>>,
    host: Arc<String>,
}

impl TransportClient {
    /// Creates an instance.
    pub fn new(address: String) -> Self {
        TransportClient {
            engine_socket: Arc::new(Mutex::new(EngineSocket::new(false))),
            host: Arc::new(address),
        }
    }

    /// Connects to the server. This includes a connection of the underlying engine.io client and
    /// afterwards an opening socket.io request.
    pub async fn connect(&self) -> Result<(), Error> {
        self.engine_socket
            .as_ref()
            .lock()
            .unwrap()
            .bind(self.host.as_ref().to_string())
            .await?;

        // construct the opening packet
        let open_packet =
            SocketPacket::new(SocketPacketId::Connect, String::from("/"), None, None, None);

        self.send(open_packet).await
    }

    /// Sends a socket.io packet to the server using the engine.io client.
    pub async fn send(&self, packet: SocketPacket) -> Result<(), Error> {
        // the packet       ü+++++++++++++++++                                                                                                                                                                                                                                                                                                          
        let engine_packet = EnginePacket::new(EnginePacketId::Message, packet.encode());

        self.engine_socket.lock().unwrap().emit(engine_packet).await
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
