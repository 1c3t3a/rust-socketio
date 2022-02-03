use std::{future::Future, pin::Pin, task::Poll};

use crate::{asynchronous::async_socket::Socket as InnerSocket, error::Result, Packet, PacketId};
use bytes::Bytes;
use futures_util::{ready, Stream};

#[derive(Clone, Debug)]
pub struct Client {
    pub(super) socket: InnerSocket,
}

impl Client {
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

    /// Check if the underlying transport client is connected.
    pub fn is_connected(&self) -> Result<bool> {
        self.socket.is_connected()
    }
}

impl Stream for Client {
    type Item = Result<Packet>;

    fn poll_next(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let packet = ready!(Pin::new(&mut self.socket.clone()).poll_next(cx));

        if let Some(result) = packet {
            match result {
                Ok(payload) => {
                    // check for the appropriate action or callback
                    ready!(
                        Pin::new(&mut Box::pin(self.socket.handle_packet(payload.clone())))
                            .poll(cx)
                    );

                    match payload.packet_id {
                        PacketId::MessageBinary | PacketId::Message => {
                            ready!(Pin::new(&mut Box::pin(
                                self.socket.handle_data(payload.data.clone())
                            ))
                            .poll(cx));
                        }
                        PacketId::Close => {
                            ready!(Pin::new(&mut Box::pin(self.socket.handle_close())).poll(cx));
                        }
                        PacketId::Open => {
                            unreachable!("Won't happen as we open the connection beforehand");
                        }
                        PacketId::Upgrade => {
                            // this is already checked during the handshake, so just do nothing here
                        }
                        PacketId::Ping => {
                            ready!(Pin::new(&mut Box::pin(self.socket.pinged())).poll(cx));
                            ready!(Pin::new(&mut Box::pin(
                                self.socket.emit(Packet::new(PacketId::Pong, Bytes::new()))
                            ))
                            .poll(cx))?;
                        }
                        PacketId::Pong => {
                            // this will never happen as the pong packet is
                            // only sent by the client
                            unreachable!();
                        }
                        PacketId::Noop => (),
                    }

                    return Poll::Ready(Some(Ok(payload)));
                }
                Err(e) => return Poll::Ready(Some(Err(e))),
            }
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod test {

    use super::*;
    use crate::{asynchronous::ClientBuilder, header::HeaderMap, packet::PacketId, Error};
    use futures_util::StreamExt;
    use native_tls::TlsConnector;
    use url::Url;

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

        socket
            .emit(Packet::new(PacketId::Message, "respond"))
            .await?;

        assert_eq!(
            socket.next().await.unwrap()?,
            Packet::new(PacketId::Message, "Roger Roger")
        );

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

        assert!(!socket.is_connected()?);

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
        let mut builder = builder(url);
        builder = builder.tls_config(crate::test::tls_connector()?);
        let socket = builder.build().await?;
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
