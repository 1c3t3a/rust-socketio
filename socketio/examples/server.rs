use futures_util::FutureExt;
use rust_socketio::{
    asynchronous::{ServerBuilder, ServerClient},
    Payload,
};
use serde_json::json;

#[tokio::main]
async fn main() {
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
    let server = ServerBuilder::new(4209).on("/", "echo", callback).build();
    server.serve().await;
}
