use std::{fmt::Debug, pin::Pin};

use crate::{
    asynchronous::{async_socket::Socket as InnerSocket, generator::StreamGenerator},
    error::Result,
    Packet,
};
use async_stream::try_stream;
use futures_util::{Stream, StreamExt};
use tokio::time::Instant;

/// An engine.io client that allows interaction with the connected engine.io
/// server. This client provides means for connecting, disconnecting and sending
/// packets to the server.
///
/// ## Note:
/// There is no need to put this Client behind an `Arc`, as the type uses `Arc`
/// internally and provides a shared state beyond all cloned instances.
#[derive(Clone)]
pub struct Client {
    pub(super) socket: InnerSocket,
    generator: StreamGenerator<Packet>,
}

impl Client {
    pub(crate) fn new(socket: InnerSocket) -> Self {
        Client {
            socket: socket.clone(),
            generator: StreamGenerator::new(Self::stream(socket)),
        }
    }

    pub async fn close(&self) -> Result<()> {
        self.socket.disconnect().await
    }

    /// Opens the connection to a specified server. The first Pong packet is sent
    /// to the server to trigger the Ping-cycle.
    pub async fn connect(&self) -> Result<()> {
        self.socket.connect().await
    }

    /// Disconnects the connection.
    pub async fn disconnect(&self) -> Result<()> {
        self.socket.disconnect().await
    }

    /// Sends a packet to the server.
    pub async fn emit(&self, packet: Packet) -> Result<()> {
        self.socket.emit(packet).await
    }

    pub(crate) async fn last_pong(&self) -> Instant {
        self.socket.last_pong().await
    }

    /// Static method that returns a generator for each element of the stream.
    fn stream(
        socket: InnerSocket,
    ) -> Pin<Box<impl Stream<Item = Result<Packet>> + 'static + Send>> {
        Box::pin(try_stream! {
            for await item in socket.clone() {
                let packet = item?;
                socket.handle_inconming_packet(packet.clone()).await?;
                yield packet;
            }
        })
    }

    /// Check if the underlying transport client is connected.
    pub fn is_connected(&self) -> bool {
        self.socket.is_connected()
    }
}

impl Stream for Client {
    type Item = Result<Packet>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.generator.poll_next_unpin(cx)
    }
}

impl Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("socket", &self.socket)
            .finish()
    }
}

#[cfg(all(test))]
mod test {

    use super::*;
    use crate::{asynchronous::ClientBuilder, header::HeaderMap, packet::PacketId, Error};
    use bytes::Bytes;
    use futures_util::StreamExt;
    use native_tls::TlsConnector;
    use url::Url;

    /// The purpose of this test is to check whether the Client is properly cloneable or not.
    /// As the documentation of the engine.io client states, the object needs to maintain it's internal
    /// state when cloned and the cloned object should reflect the same state throughout the lifetime
    /// of both objects (initial and cloned).
    #[tokio::test]
    async fn test_client_cloneable() -> Result<()> {
        let url = crate::test::engine_io_server()?;

        let mut sut = builder(url).build().await?;
        let mut cloned = sut.clone();

        sut.connect().await?;

        // when the underlying socket is connected, the
        // state should also change on the cloned one
        assert!(sut.is_connected());
        assert!(cloned.is_connected());

        // both clients should reflect the same messages.
        assert_eq!(
            sut.next().await.unwrap()?,
            Packet::new(PacketId::Message, "hello client")
        );

        sut.emit(Packet::new(PacketId::Message, "respond")).await?;

        assert_eq!(
            cloned.next().await.unwrap()?,
            Packet::new(PacketId::Message, "Roger Roger")
        );

        cloned.disconnect().await?;

        // when the underlying socket is disconnected, the
        // state should also change on the cloned one
        assert!(!sut.is_connected());
        assert!(!cloned.is_connected());

        Ok(())
    }

    #[tokio::test]
    async fn test_illegal_actions() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let mut sut = builder(url.clone()).build().await?;

        assert!(sut
            .emit(Packet::new(PacketId::Close, Bytes::new()))
            .await
            .is_err());

        sut.connect().await?;

        assert!(sut.next().await.unwrap().is_ok());

        assert!(builder(Url::parse("fake://fake.fake").unwrap())
            .build_websocket()
            .await
            .is_err());

        sut.disconnect().await?;

        Ok(())
    }

    use reqwest::header::HOST;

    use crate::packet::Packet;

    fn builder(url: Url) -> ClientBuilder {
        ClientBuilder::new(url)
            .on_open(|_| {
                Box::pin(async {
                    println!("Open event!");
                })
            })
            .on_packet(|packet| {
                Box::pin(async move {
                    println!("Received packet: {:?}", packet);
                })
            })
            .on_data(|data| {
                Box::pin(async move {
                    println!("Received data: {:?}", std::str::from_utf8(&data));
                })
            })
            .on_close(|_| {
                Box::pin(async {
                    println!("Close event!");
                })
            })
            .on_error(|error| {
                Box::pin(async move {
                    println!("Error {}", error);
                })
            })
    }

    async fn test_connection(socket: Client) -> Result<()> {
        let mut socket = socket;

        socket.connect().await.unwrap();

        assert_eq!(
            socket.next().await.unwrap()?,
            Packet::new(PacketId::Message, "hello client")
        );
        println!("received msg, abut to send");

        socket
            .emit(Packet::new(PacketId::Message, "respond"))
            .await?;

        println!("send msg");

        assert_eq!(
            socket.next().await.unwrap()?,
            Packet::new(PacketId::Message, "Roger Roger")
        );
        println!("received 2");

        socket.close().await
    }

    #[tokio::test]
    async fn test_connection_long() -> Result<()> {
        // Long lived socket to receive pings
        let url = crate::test::engine_io_server()?;
        let mut socket = builder(url).build().await?;

        socket.connect().await?;

        // hello client
        assert!(matches!(
            socket.next().await.unwrap()?,
            Packet {
                packet_id: PacketId::Message,
                ..
            }
        ));
        // Ping
        assert!(matches!(
            socket.next().await.unwrap()?,
            Packet {
                packet_id: PacketId::Ping,
                ..
            }
        ));

        socket.disconnect().await?;

        assert!(!socket.is_connected());

        Ok(())
    }

    #[tokio::test]
    async fn test_connection_dynamic() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build().await?;
        test_connection(socket).await?;

        let url = crate::test::engine_io_polling_server()?;
        let socket = builder(url).build().await?;
        test_connection(socket).await
    }

    #[tokio::test]
    async fn test_connection_fallback() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build_with_fallback().await?;
        test_connection(socket).await?;

        let url = crate::test::engine_io_polling_server()?;
        let socket = builder(url).build_with_fallback().await?;
        test_connection(socket).await
    }

    #[tokio::test]
    async fn test_connection_dynamic_secure() -> Result<()> {
        let url = crate::test::engine_io_server_secure()?;
        let mut socket_builder = builder(url);
        socket_builder = socket_builder.tls_config(crate::test::tls_connector()?);
        let socket = socket_builder.build().await?;
        test_connection(socket).await
    }

    #[tokio::test]
    async fn test_connection_polling() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let socket = builder(url).build_polling().await?;
        test_connection(socket).await
    }

    #[tokio::test]
    async fn test_connection_wss() -> Result<()> {
        let url = crate::test::engine_io_polling_server()?;
        assert!(builder(url).build_websocket_with_upgrade().await.is_err());

        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        let mut url = crate::test::engine_io_server_secure()?;

        let mut headers = HeaderMap::default();
        headers.insert(HOST, host);
        let mut builder = builder(url.clone());

        builder = builder.tls_config(crate::test::tls_connector()?);
        builder = builder.headers(headers.clone());
        let socket = builder.clone().build_websocket_with_upgrade().await?;

        test_connection(socket).await?;

        let socket = builder.build_websocket().await?;

        test_connection(socket).await?;

        url.set_scheme("wss").unwrap();

        let builder = self::builder(url)
            .tls_config(crate::test::tls_connector()?)
            .headers(headers);
        let socket = builder.clone().build_websocket().await?;

        test_connection(socket).await?;

        assert!(builder.build_websocket_with_upgrade().await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_connection_ws() -> Result<()> {
        let url = crate::test::engine_io_polling_server()?;
        assert!(builder(url.clone()).build_websocket().await.is_err());
        assert!(builder(url).build_websocket_with_upgrade().await.is_err());

        let mut url = crate::test::engine_io_server()?;

        let builder = builder(url.clone());
        let socket = builder.clone().build_websocket().await?;
        test_connection(socket).await?;

        let socket = builder.build_websocket_with_upgrade().await?;
        test_connection(socket).await?;

        url.set_scheme("ws").unwrap();

        let builder = self::builder(url);
        let socket = builder.clone().build_websocket().await?;

        test_connection(socket).await?;

        assert!(builder.build_websocket_with_upgrade().await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_open_invariants() -> Result<()> {
        let url = crate::test::engine_io_server()?;
        let illegal_url = "this is illegal";

        assert!(Url::parse(illegal_url).is_err());

        let invalid_protocol = "file:///tmp/foo";
        assert!(builder(Url::parse(invalid_protocol).unwrap())
            .build()
            .await
            .is_err());

        let sut = builder(url.clone()).build().await?;
        let _error = sut
            .emit(Packet::new(PacketId::Close, Bytes::new()))
            .await
            .expect_err("error");
        assert!(matches!(Error::IllegalActionBeforeOpen(), _error));

        // test missing match arm in socket constructor
        let mut headers = HeaderMap::default();
        let host =
            std::env::var("ENGINE_IO_SECURE_HOST").unwrap_or_else(|_| "localhost".to_owned());
        headers.insert(HOST, host);

        let _ = builder(url.clone())
            .tls_config(
                TlsConnector::builder()
                    .danger_accept_invalid_certs(true)
                    .build()
                    .unwrap(),
            )
            .build()
            .await?;
        let _ = builder(url).headers(headers).build().await?;
        Ok(())
    }
}
