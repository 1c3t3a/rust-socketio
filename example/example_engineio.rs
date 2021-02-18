fn main() {
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
        .unwrap();

    socket.emit(Packet::new(
            PacketId::Message,
            "Hello World".to_owned().into_bytes(),
    )).unwrap();

    socket.emit(Packet::new(
            PacketId::Message,
            "Hello World2".to_owned().into_bytes(),
    )).unwrap();

    socket.emit(Packet::new(PacketId::Pong, Vec::new())).unwrap();

    socket.emit(Packet::new(PacketId::Ping, Vec::new())).unwrap();

    sleep(Duration::from_secs(26));

    socket.emit(Packet::new(
        PacketId::Message,
        "Hello World3".to_owned().into_bytes(),
    )).unwrap();
}
