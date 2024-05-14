use rust_socketio::{
    client::Client as SocketIOClient, ClientBuilder as SocketIOClientBuilder,
    Error as SocketIOError, Payload, RawClient,
};
use serde_json::json;
use std::sync::{mpsc, Arc};
use std::thread::sleep;
use std::time::Duration;

struct ComplexData {
    /// There should be many more fields below in real life,
    /// probaly wrapped in Arc<Mutex<T>> if you're writing a more serious client.
    data: String,
}

struct TransmitterClient {
    client: SocketIOClient,
    receiver: mpsc::Receiver<String>,
    complex: ComplexData,
}

impl TransmitterClient {
    fn connect(url: &str) -> Result<Self, SocketIOError> {
        let (sender, receiver) = mpsc::channel::<String>();

        let client = SocketIOClientBuilder::new(url)
            .namespace("/admin")
            .on(
                "test",
                |payload: Payload, socket: RawClient| match payload {
                    Payload::Text(values) => {
                        if let Some(value) = values.first() {
                            if value.is_string() {
                                socket
                                    .try_transmitter::<mpsc::Sender<String>>()
                                    .map_or_else(
                                        |err| eprintln!("{}", err),
                                        |tx| {
                                            tx.send(String::from(value.as_str().unwrap()))
                                                .map_or_else(
                                                    |err| eprintln!("{}", err),
                                                    |_| println!("Data transmitted successfully"),
                                                );
                                        },
                                    );
                            }
                        }
                    }
                    Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
                    #[allow(deprecated)]
                    Payload::String(str) => println!("Received: {}", str),
                },
            )
            .on("error", |err, _| eprintln!("Error: {:#?}", err))
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
                    Ok(complex_data) => {
                        // In the real world the data is probably a serialized json_rpc object
                        // or some other complex data layer which needs complex business and derserialization logic.
                        // Best to do that here, and not inside those restrictive callbacks.
                        self.complex.data = complex_data;
                        Some(self.complex.data.clone())
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
