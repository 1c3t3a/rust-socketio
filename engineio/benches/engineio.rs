use criterion::{criterion_group, criterion_main};
use native_tls::Certificate;
use native_tls::TlsConnector;
use rust_engineio::error::Error;
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

/// sync benches

#[cfg(not(feature = "async"))]
pub mod tests {
    use bytes::Bytes;
    use reqwest::Url;
    use rust_engineio::{Client, ClientBuilder, Error, Packet, PacketId};

    use crate::tls_connector;

    pub fn engine_io_socket_build(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url).build()
    }

    pub fn engine_io_socket_build_polling(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url).build_polling()
    }

    pub fn engine_io_socket_build_polling_secure(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url)
            .tls_config(tls_connector()?)
            .build_polling()
    }

    pub fn engine_io_socket_build_websocket(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url).build_websocket()
    }

    pub fn engine_io_socket_build_websocket_secure(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url)
            .tls_config(tls_connector()?)
            .build_websocket()
    }

    pub fn engine_io_packet() -> Packet {
        Packet::new(PacketId::Message, Bytes::from("hello world"))
    }

    pub fn engine_io_emit(socket: &Client, packet: Packet) -> Result<(), Error> {
        socket.emit(packet)
    }
}

#[cfg(not(feature = "async"))]
mod criterion_wrappers {
    use criterion::{black_box, Criterion};

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

/// async benches

#[cfg(feature = "async")]
pub mod tests {
    use bytes::Bytes;
    use rust_engineio::{
        asynchronous::{Client, ClientBuilder},
        Error, Packet, PacketId,
    };
    use url::Url;

    use crate::tls_connector;

    pub async fn engine_io_socket_build(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url).build().await
    }

    pub async fn engine_io_socket_build_polling(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url).build_polling().await
    }

    pub async fn engine_io_socket_build_polling_secure(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url)
            .tls_config(tls_connector()?)
            .build_polling()
            .await
    }

    pub async fn engine_io_socket_build_websocket(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url).build_websocket().await
    }

    pub async fn engine_io_socket_build_websocket_secure(url: Url) -> Result<Client, Error> {
        ClientBuilder::new(url)
            .tls_config(tls_connector()?)
            .build_websocket()
            .await
    }

    pub fn engine_io_packet() -> Packet {
        Packet::new(PacketId::Message, Bytes::from("hello world"))
    }

    pub async fn engine_io_emit(socket: &Client, packet: Packet) -> Result<(), Error> {
        socket.emit(packet).await
    }
}

#[cfg(feature = "async")]
mod criterion_wrappers {
    use std::sync::Arc;

    use bytes::Bytes;
    use criterion::{black_box, Criterion};
    use lazy_static::lazy_static;
    use rust_engineio::{Packet, PacketId};
    use tokio::runtime::{Builder, Runtime};

    use super::tests::{
        engine_io_emit, engine_io_packet, engine_io_socket_build, engine_io_socket_build_polling,
        engine_io_socket_build_polling_secure, engine_io_socket_build_websocket,
        engine_io_socket_build_websocket_secure,
    };
    use super::util::{engine_io_url, engine_io_url_secure};

    lazy_static! {
        static ref RUNTIME: Arc<Runtime> =
            Arc::new(Builder::new_multi_thread().enable_all().build().unwrap());
    }

    pub fn criterion_engine_io_socket_build(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        c.bench_function("engine io build", move |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_socket_build(black_box(url.clone()))
                    .await
                    .unwrap()
                    .close()
                    .await
            })
        });
    }

    pub fn criterion_engine_io_socket_build_polling(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        c.bench_function("engine io build polling", move |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_socket_build_polling(black_box(url.clone()))
                    .await
                    .unwrap()
                    .close()
                    .await
            })
        });
    }

    pub fn criterion_engine_io_socket_build_polling_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        c.bench_function("engine io build polling secure", move |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_socket_build_polling_secure(black_box(url.clone()))
                    .await
                    .unwrap()
                    .close()
                    .await
            })
        });
    }

    pub fn criterion_engine_io_socket_build_websocket(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        c.bench_function("engine io build websocket", move |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_socket_build_websocket(black_box(url.clone()))
                    .await
                    .unwrap()
                    .close()
                    .await
            })
        });
    }

    pub fn criterion_engine_io_socket_build_websocket_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        c.bench_function("engine io build websocket secure", move |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_socket_build_websocket_secure(black_box(url.clone()))
                    .await
                    .unwrap()
                    .close()
                    .await
            })
        });
    }

    pub fn criterion_engine_io_packet(c: &mut Criterion) {
        c.bench_function("engine io packet", move |b| {
            b.iter(|| Packet::new(PacketId::Message, Bytes::from("hello world")))
        });
    }

    pub fn criterion_engine_io_emit_polling(c: &mut Criterion) {
        let url = engine_io_url().unwrap();
        let socket = RUNTIME.block_on(async {
            let socket = engine_io_socket_build_polling(url).await.unwrap();
            socket.connect().await.unwrap();
            socket
        });

        let packet = engine_io_packet();

        c.bench_function("engine io polling emit", |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_emit(black_box(&socket), black_box(packet.clone()))
                    .await
                    .unwrap()
            })
        });
    }

    pub fn criterion_engine_io_emit_polling_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();

        let socket = RUNTIME.block_on(async {
            let socket = engine_io_socket_build_polling_secure(url).await.unwrap();
            socket.connect().await.unwrap();
            socket
        });

        let packet = engine_io_packet();

        c.bench_function("engine io polling secure emit", |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_emit(black_box(&socket), black_box(packet.clone()))
                    .await
                    .unwrap()
            })
        });
    }

    pub fn criterion_engine_io_emit_websocket(c: &mut Criterion) {
        let url = engine_io_url().unwrap();

        let socket = RUNTIME.block_on(async {
            let socket = engine_io_socket_build_websocket(url).await.unwrap();
            socket.connect().await.unwrap();
            socket
        });

        let packet = engine_io_packet();

        c.bench_function("engine io websocket emit", |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_emit(black_box(&socket), black_box(packet.clone()))
                    .await
                    .unwrap()
            })
        });
    }

    pub fn criterion_engine_io_emit_websocket_secure(c: &mut Criterion) {
        let url = engine_io_url_secure().unwrap();
        let socket = RUNTIME.block_on(async {
            let socket = engine_io_socket_build_websocket_secure(url).await.unwrap();
            socket.connect().await.unwrap();
            socket
        });

        let packet = engine_io_packet();

        c.bench_function("engine io websocket secure emit", |b| {
            b.to_async(RUNTIME.as_ref()).iter(|| async {
                engine_io_emit(black_box(&socket), black_box(packet.clone()))
                    .await
                    .unwrap()
            })
        });
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
