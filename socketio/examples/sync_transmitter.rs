use rust_socketio::{
    client::Client as SocketIOClient, ClientBuilder as SocketIOClientBuilder,
    Error as SocketIOError, Payload, RawClient,
};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc};
use std::thread::sleep;
use std::time::Duration;

type JsonValues = Vec<Value>;

fn test_event_handler(payload: Payload, socket: RawClient) {
    if let Payload::Text(values) = payload {
        match socket.try_transmitter::<mpsc::Sender<JsonValues>>() {
            Ok(tx) => {
                tx.send(values.to_owned()).map_or_else(
                    |err| eprintln!("{}", err),
                    |_| println!("Data transmitted successfully"),
                );
            }
            Err(err) => {
                eprintln!("{}", err);
            }
        }
    }
}

fn error_event_handler(payload: Payload, _: RawClient) {
    eprintln!("Error: {:#?}", payload);
}

struct ComplexData {
    /// There should be many more fields below in real life,
    /// probaly wrapped in Arc<Mutex<T>> if you're writing a more serious client.
    data: String,
}

struct TransmitterClient {
    client: SocketIOClient,
    receiver: mpsc::Receiver<JsonValues>,
    complex: ComplexData,
}

impl TransmitterClient {
    fn connect(url: &str) -> Result<Self, SocketIOError> {
        let (sender, receiver) = mpsc::channel::<JsonValues>();

        let client = SocketIOClientBuilder::new(url)
            .namespace("/admin")
            .on("test", test_event_handler)
            .on("error", error_event_handler)
            .transmitter(Arc::new(sender))
            .connect()?;

        Ok(Self {
            client,
            receiver,
            complex: ComplexData {
                data: "".to_string(),
            },
        })
    }

    fn get_test(&mut self) -> Option<String> {
        match self.client.emit("test", json!({"got ack": true})) {
            Ok(_) => {
                match self.receiver.recv() {
                    Ok(values) => {
                        // Json deserialization and parsing business logic should be implemented
                        // here to avoid over-complicating the handler callbacks.
                        if let Some(value) = values.first() {
                            if value.is_string() {
                                self.complex.data = String::from(value.as_str().unwrap());
                                return Some(self.complex.data.clone());
                            }
                        }
                        None
                    }
                    Err(err) => {
                        eprintln!("Transmission buffer is probably full: {}", err);
                        None
                    }
                }
            }
            Err(err) => {
                eprintln!("Server unreachable: {}", err);
                None
            }
        }
    }
}

fn main() {
    match TransmitterClient::connect("http://localhost:4200/") {
        Ok(mut client) => {
            if let Some(test_data) = client.get_test() {
                println!("test event data from internal transmitter: {}", test_data);
            }
        }
        Err(err) => {
            eprintln!("Failed to connect to server: {}", err);
        }
    }

    sleep(Duration::from_secs(2));
}
