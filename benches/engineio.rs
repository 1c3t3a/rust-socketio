use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rust_socketio::engineio::{
    packet::{Packet, PacketId},
    transport::TransportClient,
};
use rust_socketio::error::Error;

fn engine_io_emit(socket: &TransportClient, packet: Packet) -> Result<(), Error> {
    let mut i = 0;
    while i < 100 {
        i += 1;
        socket.emit(packet.clone(), false)?;
    }
    Ok(())
}

fn engine_io_url() -> Result<String, Error> {
    const SERVER_URL: &str = "http://localhost:4201";
    let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());
    Ok(url)
}

fn engine_io_socket_build(url: String) -> TransportClient {
    let mut client = TransportClient::new(true, None, None);
    client.open(url).unwrap();
    client
}

fn engine_io_packet() -> Packet {
    Packet::new(PacketId::Message, Bytes::from("hello world"))
}

fn criterion_engine_io_socket_build(c: &mut Criterion) {
    let url = engine_io_url().unwrap();

    c.bench_function("engine io build", |b| {
        b.iter(|| engine_io_socket_build(black_box(url.clone())))
    });
}

fn criterion_engine_io_packet(c: &mut Criterion) {
    c.bench_function("engine io emit", |b| b.iter(|| engine_io_packet()));
}

fn criterion_engine_io_emit(c: &mut Criterion) {
    let url = engine_io_url().unwrap();
    let socket = engine_io_socket_build(url);
    let packet = engine_io_packet();

    c.bench_function("engine io emit", |b| {
        b.iter(|| engine_io_emit(black_box(&socket), black_box(packet.clone())).unwrap())
    });
}

criterion_group!(
    benches,
    criterion_engine_io_socket_build,
    criterion_engine_io_packet,
    criterion_engine_io_emit
);
criterion_main!(benches);
