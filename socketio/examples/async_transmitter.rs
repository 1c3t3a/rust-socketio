use futures_util::FutureExt;
use rust_socketio::{
    asynchronous::{Client as SocketIOClient, ClientBuilder as SocketIOClientBuilder},
    Error as SocketIOError, Payload,
};
use serde_json::json;
use std::sync::{mpsc, Arc};
use std::time::Duration;
use tokio::time::sleep;

struct ComplexData {
    /// There should be many more fields below in real life,
    /// probaly wrapped in Arc<Mutex<T>> if you're writing a more serious client.
    data: String,
}

struct TransmitterClient {
    receiver: mpsc::Receiver<String>,
    complex: ComplexData,
    client: SocketIOClient,
}

impl TransmitterClient {
    async fn connect(url: &str) -> Result<Self, SocketIOError> {
        let (sender, receiver) = mpsc::channel::<String>();

        let client = SocketIOClientBuilder::new(url)
            .namespace("/admin")
            .on("test", |payload: Payload, socket: SocketIOClient| {
                async move {
                    match payload {
                        Payload::Text(values) => {
                            if let Some(value) = values.first() {
                                if value.is_string() {
                                    let result = socket.try_transitter::<mpsc::Sender<String>>();

                                    result
                                        .map(|transmitter| {
                                            transmitter.send(String::from(value.as_str().unwrap()))
                                        })
                                        .map_err(|err| eprintln!("{}", err))
                                        .ok();
                                }
                            }
                        }
                        Payload::Binary(_bin_data) => println!(),
                        #[allow(deprecated)]
                        Payload::String(str) => println!("Received: {}", str),
                    }
                }
                .boxed()
            })
            .on("error", |err, _| {
                async move { eprintln!("Error: {:#?}", err) }.boxed()
            })
            .transmitter(Arc::new(sender))
            .connect()
            .await?;

        Ok(Self {
            client,
            receiver,
            complex: ComplexData {
                data: "".to_string(),
            },
        })
    }

    async fn get_test(&mut self) -> Option<String> {
        match self.client.emit("test", json!({"got ack": true})).await {
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

#[tokio::main]
async fn main() {
    match TransmitterClient::connect("http://localhost:4200/").await {
        Ok(mut client) => {
            if let Some(test_data) = client.get_test().await {
                println!("test event data from internal transmitter: {}", test_data);
            }
        }
        Err(err) => {
            eprintln!("{}", err);
        }
    }

    // Wait so we can see our response
    sleep(Duration::from_secs(2)).await;
}
