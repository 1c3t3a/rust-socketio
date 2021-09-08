use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use native_tls::Certificate;
use native_tls::TlsConnector;
use rust_engineio::error::Error;
use rust_engineio::{
    packet::{Packet, PacketId},
    {Socket, SocketBuilder},
};
use std::fs::File;
use std::io::Read;
use url::Url;

pub use criterion_wrappers::*;
pub use tests::*;
pub use util::*;

pub mod util {
    use super::*;
    pub fn engine_io_url() -> Result<Url, Error> {
        let url = std::env::var("ENGINE_IO_SERVER")
            .unwrap_or_else(|_| "http://localhost:4201".to_owned());
        Ok(Url::parse(&url)?)
    }

    pub fn engine_io_url_secure() -> Result<Url, Error> {
        let url = std::env::var("ENGINE_IO_SECURE_SERVER")
            .unwrap_or_else(|_| "https://localhost:4202".to_owned());
        Ok(Url::parse(&url)?)
    }

    pub fn tls_connector() -> Result<TlsConnector, Error> {
        let cert_path = "../".to_owned()
            + &std::env::var("CA_CERT_PATH").unwrap_or_else(|_| "ci/cert/ca.crt".to_owned());
        let mut cert_file = File::open(cert_path)?;
        let mut buf = vec![];
        cert_file.read_to_end(&mut buf)?;
        let cert: Certificate = Certificate::from_pem(&buf[..]).unwrap();
        Ok(TlsConnector::builder()
            // ONLY USE FOR TESTING!
            .danger_accept_invalid_hostnames(true)
            .add_root_certificate(cert)
            .build()
            .unwrap())
    }
}

pub mod tests {
    use super::*;
    pub fn engine_io_socket_build(url: Url) -> Result<Socket, Error> {
        SocketBuilder::new(url).build()
    }

    pub fn engine_io_socket_build_polling(url: Url) -> Result<Socket, Error> {
        SocketBuilder::new(url).build_polling()
    }

    pub fn engine_io_socket_build_polling_secure(url: Url) -> Result<Socket, Error> {
        SocketBuilder::new(url)
            .tls_config(tls_connector()?)
            .build_polling()
    }

    pub fn engine_io_socket_build_websocket(url: Url) -> Result<Socket, Error> {
        SocketBuilder::new(url).build_websocket()
    }

    pub fn engine_io_socket_build_websocket_secure(url: Url) -> Result<Socket, Error> {
        SocketBuilder::new(url)
            .tls_config(tls_connector()?)
            .build_websocket()
    }

    pub fn engine_io_packet() -> Packet {
        Packet::new(PacketId::Message, Bytes::from("hello world"))
    }

    pub fn engine_io_emit(socket: &Socket, packet: Packet) -> Result<(), Error> {
        socket.emit(packet)
    }
}
mod criterion_wrappers {
    use super::*;

    pub fn criterion_engine_io_socket_build(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        c.bench_function("engine io build", |b| {
            b.iter(|| {
                engine_io_socket_build(black_box(url.clone()))
                    .unwrap()
                    .close()
            })
        });
    }

    pub fn criterion_engine_io_socket_build_polling(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        c.bench_function("engine io build polling", |b| {
            b.iter(|| {
                engine_io_socket_build_polling(black_box(url.clone()))
                    .unwrap()
                    .close()
            })
        });
    }

    pub fn criterion_engine_io_socket_build_polling_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        c.bench_function("engine io build polling secure", |b| {
            b.iter(|| {
                engine_io_socket_build_polling_secure(black_box(url.clone()))
                    .unwrap()
                    .close()
            })
        });
    }

    pub fn criterion_engine_io_socket_build_websocket(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        c.bench_function("engine io build websocket", |b| {
            b.iter(|| {
                engine_io_socket_build_websocket(black_box(url.clone()))
                    .unwrap()
                    .close()
            })
        });
    }

    pub fn criterion_engine_io_socket_build_websocket_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        c.bench_function("engine io build websocket secure", |b| {
            b.iter(|| {
                engine_io_socket_build_websocket_secure(black_box(url.clone()))
                    .unwrap()
                    .close()
            })
        });
    }

    pub fn criterion_engine_io_packet(c: &mut Criterion) {
        c.bench_function("engine io packet", |b| b.iter(|| engine_io_packet()));
    }

    pub fn criterion_engine_io_emit_polling(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        let socket = engine_io_socket_build_polling(url).unwrap();
        socket.connect().unwrap();
        let packet = engine_io_packet();

        c.bench_function("engine io polling emit", |b| {
            b.iter(|| engine_io_emit(black_box(&socket), black_box(packet.clone())).unwrap())
        });
        socket.close().unwrap();
    }

    pub fn criterion_engine_io_emit_polling_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        let socket = engine_io_socket_build_polling_secure(url).unwrap();
        socket.connect().unwrap();
        let packet = engine_io_packet();

        c.bench_function("engine io polling secure emit", |b| {
            b.iter(|| engine_io_emit(black_box(&socket), black_box(packet.clone())).unwrap())
        });
        socket.close().unwrap();
    }

    pub fn criterion_engine_io_emit_websocket(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        let socket = engine_io_socket_build_websocket(url).unwrap();
        socket.connect().unwrap();
        let packet = engine_io_packet();

        c.bench_function("engine io websocket emit", |b| {
            b.iter(|| engine_io_emit(black_box(&socket), black_box(packet.clone())).unwrap())
        });
        socket.close().unwrap();
    }

    pub fn criterion_engine_io_emit_websocket_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        let socket = engine_io_socket_build_websocket_secure(url).unwrap();
        socket.connect().unwrap();
        let packet = engine_io_packet();

        c.bench_function("engine io websocket secure emit", |b| {
            b.iter(|| engine_io_emit(black_box(&socket), black_box(packet.clone())).unwrap())
        });
        socket.close().unwrap();
    }
}

criterion_group!(
    benches,
    criterion_engine_io_socket_build_polling,
    criterion_engine_io_socket_build_polling_secure,
    criterion_engine_io_socket_build_websocket,
    criterion_engine_io_socket_build_websocket_secure,
    criterion_engine_io_socket_build,
    criterion_engine_io_packet,
    criterion_engine_io_emit_polling,
    criterion_engine_io_emit_polling_secure,
    criterion_engine_io_emit_websocket,
    criterion_engine_io_emit_websocket_secure
);
criterion_main!(benches);
