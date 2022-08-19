use rust_socketio::{Client, ClientBuilder, Event, Payload};
use serde_json::json;

fn handle_foo(payload: Payload, socket: Client) -> () {
    socket.emit("bar", payload).expect("Server unreachable")
}

fn main() {
    // define a callback which is called when a payload is received
    // this callback gets the payload as well as an instance of the
    // socket to communicate with the server
    let handle_test = |payload: Payload, socket: Client| {
        match payload {
            Payload::String(str) => println!("Received string: {}", str),
            Payload::Binary(bin_data) => println!("Received bytes: {:#?}", bin_data),
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
        .on("error", |err, _| eprintln!("Error: {:#?}", err))
        // Function call with signature (payload: Payload, socket: Client) -> ()
        .on("foo", handle_foo)
        // Reserved event names are case insensitive
        .on("oPeN", |_, _| println!("Connected"))
        // Custom names are case sensitive
        .on("Test", |_, _| println!("TesT received"))
        // Event specified by enum
        .on(Event::Close, |_, socket| {
            println!("Socket Closed");
            let r = socket.emit("message", json!({"foo": "Hello server"}));
            assert!(r.is_err());
        })
        .connect()
        .expect("Connection failed");

    // use the socket

    socket.disconnect().expect("Disconnect failed")
}
