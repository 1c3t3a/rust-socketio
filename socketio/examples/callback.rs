use rust_socketio::{ClientBuilder, Event, Payload, RawClient};
use serde_json::json;

fn handle_foo(payload: Payload, socket: RawClient, _id: Option<i32>) -> () {
    socket.emit("bar", payload).expect("Server unreachable")
}

fn main() {
    // define a callback which is called when a payload is received
    // this callback gets the payload as well as an instance of the
    // socket to communicate with the server
    let handle_test = |payload: Payload, socket: RawClient, _id: Option<i32>| {
        match payload {
            Payload::Text(text) => println!("Received json: {:#?}", text),
            Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
            #[allow(deprecated)]
            // Use Payload::Text instead
            Payload::String(str) => println!("Received string: {}", str),
        }
        socket
            .emit("test", json!({"got ack": true}))
            .expect("Server unreachable")
    };

    // get a socket that is connected to the admin namespace
    let socket = ClientBuilder::new("http://localhost:4200")
        .namespace("/admin")
        // Saved closure
        .on("test", handle_test)
        // Inline closure
        .on("error", |err, _, _| eprintln!("Error: {:#?}", err))
        // Function call with signature (payload: Payload, socket: RawClient) -> ()
        .on("foo", handle_foo)
        // Reserved event names are case insensitive
        .on("oPeN", |_, _, _| println!("Connected"))
        // Custom names are case sensitive
        .on("Test", |_, _, _| println!("TesT received"))
        // Event specified by enum
        .on(Event::Close, |_, socket, _| {
            println!("Socket Closed");
            socket
                .emit("message", json!({"foo": "Hello server"}))
                .expect("Error emitting");
        })
        .connect()
        .expect("Connection failed");

    // use the socket

    socket.disconnect().expect("Disconnect failed")
}
