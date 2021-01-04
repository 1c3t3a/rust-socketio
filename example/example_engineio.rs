
#[tokio::main]
async fn main() {
    let mut socket = EngineSocket::new(true);

    socket.on_open(|_| {
        println!("Open event!");
    }).unwrap();

    socket.on_close(|_| {
        println!("Close event!");
    }).unwrap();

    socket.on_packet(|packet| {
        println!("Received packet: {:?}", packet);
    }).unwrap();

    socket.on_data(|data| {
        println!("Received packet: {:?}", std::str::from_utf8(&data));
    }).unwrap();

    socket
        .bind(String::from("http://localhost:4200"))
        .await
        .unwrap();

    socket.emit(Packet::new(
            PacketId::Message,
            "Hello World".to_string().into_bytes(),
    )).await.unwrap();

    socket.emit(Packet::new(
            PacketId::Message,
            "Hello World2".to_string().into_bytes(),
    )).await.unwrap();

    socket.emit(Packet::new(PacketId::Pong, Vec::new())).await.unwrap();

    socket.emit(Packet::new(PacketId::Ping, Vec::new())).await.unwrap();

    tokio::time::delay_for(Duration::from_secs(26)).await;

    socket.emit(Packet::new(
        PacketId::Message,
        "Hello World3".to_string().into_bytes(),
    )).await.unwrap();
}
