use std::sync::{atomic::AtomicBool, Arc};

use crate::packet::{decode_payload, encode_payload, Packet};
use serde::{Deserialize, Serialize};
use std::net::TcpStream;
use websocket::sync::Client;
use websocket::{ClientBuilder, Message, OwnedMessage};

enum TransportType {
    Websocket(Option<Client<TcpStream>>),
}

struct TransportClient {
    transport: TransportType,
    // do we might need a lock here?
    on_error: Arc<Option<Box<dyn Fn(String) -> ()>>>,
    on_open: Arc<Option<Box<dyn Fn() -> ()>>>,
    on_close: Arc<Option<Box<dyn Fn() -> ()>>>,
    on_data: Arc<Option<Box<dyn Fn(Vec<u8>) -> ()>>>,
    on_packet: Arc<Option<Box<dyn Fn(Packet) -> ()>>>,
    ready_state: Arc<AtomicBool>,
    connection_data: Option<HandshakeData>,
}

#[derive(Serialize, Deserialize, Debug)]
struct HandshakeData {
    sid: String,
    upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    ping_interval: i32,
    #[serde(rename = "pingTimeout")]
    ping_timeout: i32,
}

impl TransportClient {
    pub fn new() -> Self {
        TransportClient {
            transport: TransportType::Websocket(None),
            on_error: Arc::new(None),
            on_open: Arc::new(None),
            on_close: Arc::new(None),
            on_data: Arc::new(None),
            on_packet: Arc::new(None),
            ready_state: Arc::new(AtomicBool::default()),
            connection_data: None,
        }
    }

    pub fn open(&mut self, address: String) {
        match &self.transport {
            TransportType::Websocket(socket) => {
                if let None = socket {
                    let address = address + "/engine.io/?EIO=4&transport=websocket";
                    let mut ws_client = ClientBuilder::new(&address[..])
                        .unwrap()
                        .connect_insecure()
                        .unwrap();

                    ws_client
                        .send_message(&Message::binary(Vec::new()))
                        .unwrap();

                    if let OwnedMessage::Text(response) = ws_client.recv_message().unwrap() {
                        self.connection_data =
                            Some(serde_json::from_str(dbg!(&response[1..])).unwrap());
                    }

                    self.transport = TransportType::Websocket(Some(ws_client));

                    if let Some(function) = self.on_open.as_ref() {
                        // call the callback for the connection
                        function();
                    }
                }
            }
        }
    }

    pub fn emit(&mut self, packet: Packet) {
        match &mut self.transport {
            TransportType::Websocket(socket) => {
                if let Some(socket) = socket.as_mut() {
                    let data = encode_payload(vec![packet]);
                    socket.send_message(&Message::binary(data)).unwrap();

                    let response = socket.recv_message().unwrap();
                    if let OwnedMessage::Text(response) = response {
                        println!("{:?}", decode_payload(response.into_bytes().to_vec()));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::packet::PacketId;

    use super::*;
    #[test]
    fn test_connection() {
        let mut socket = TransportClient::new();

        socket.open("ws://localhost:4200".to_owned());

        socket.emit(Packet::new(
            PacketId::Message,
            "Hello".to_string().into_bytes(),
        ));
    }
}
