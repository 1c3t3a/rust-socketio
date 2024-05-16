use futures_util::future::{BoxFuture, FutureExt};
use rust_socketio::{
    asynchronous::{Client as SocketIOClient, ClientBuilder as SocketIOClientBuilder},
    Error as SocketIOError, Payload,
};
use serde_json::{json, Value};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tokio::time::sleep;

type JsonValues = Vec<Value>;

fn test_event_handler<'event>(payload: Payload, socket: SocketIOClient) -> BoxFuture<'event, ()> {
    async move {
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
    .boxed()
}

fn error_event_handler<'event>(payload: Payload, _: SocketIOClient) -> BoxFuture<'event, ()> {
    async move { eprintln!("Error: {:#?}", payload) }.boxed()
}

struct ComplexData {
    /// There should be many more fields below in real life,
    /// probaly wrapped in Arc<Mutex<T>> if you're writing a more serious client.
    data: String,
}

struct TransmitterClient {
    receiver: mpsc::Receiver<JsonValues>,
    complex: ComplexData,
    client: SocketIOClient,
}

impl TransmitterClient {
    async fn connect(url: &str) -> Result<Self, SocketIOError> {
        let (sender, receiver) = mpsc::channel::<JsonValues>();

        let client = SocketIOClientBuilder::new(url)
            .namespace("/admin")
            .on("test", test_event_handler)
            .on("error", error_event_handler)
            .transmitter(Arc::new(sender))
            .connect()
            .await?;

        Ok(Self {
            client,
            receiver,
            complex: ComplexData {
                data: String::from(""),
            },
        })
    }

    async fn get_test(&mut self) -> Option<String> {
        match self.client.emit("test", json!({"got ack": true})).await {
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

#[tokio::main]
async fn main() {
    match TransmitterClient::connect("http://localhost:4200/").await {
        Ok(mut client) => {
            if let Some(test_data) = client.get_test().await {
                println!("test event data from internal transmitter: {}", test_data);
            }
        }
        Err(err) => {
            eprintln!("Failed to connect to server: {}", err);
        }
    }

    sleep(Duration::from_secs(2)).await;
}
