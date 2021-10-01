use crate::error::Error;
use crate::error::Result;
use crate::header::HeaderMap;
use crate::packet::Packet;
use crate::packet::PacketId;
use crate::transport::Transport;
use bytes::{BufMut, Bytes, BytesMut};
use native_tls::TlsConnector;
use tungstenite::Message;
use tungstenite::client_tls_with_config;
use tungstenite::connect;
use url::Url;
use tungstenite::WebSocket;
use tungstenite::stream::MaybeTlsStream;
use std::borrow::Cow;
use std::convert::TryFrom;
use std::convert::TryInto;
use std::net::TcpStream;
use std::str::from_utf8;
use std::sync::{Arc, Mutex, RwLock};
use tungstenite::client::IntoClientRequest;
use tungstenite::Connector::NativeTls;

#[derive(Clone)]
pub struct WebsocketTransport {
    client: Arc<Mutex<WebSocket<MaybeTlsStream<TcpStream>>>>,
    base_url: Arc<RwLock<url::Url>>,
}

impl WebsocketTransport {
    /// Creates an instance of `WebsocketSecureTransport`.
    pub fn new(
        base_url: Url,
        tls_config: Option<TlsConnector>,
        headers: Option<HeaderMap>,
    ) -> Result<Self> {
        let mut url = base_url;
        match url.scheme() {
            "http" => url.set_scheme("ws").unwrap(),
            "https" => url.set_scheme("wss").unwrap(),
            _ => ()
        };
        url.query_pairs_mut().append_pair("transport", "websocket");
        let mut request = url.clone().into_client_request()?;
        if let Some(headers) = headers {
            let output_headers = request.headers_mut();
            for (key, value) in headers.into_iter() {
                output_headers.append(reqwest::header::HeaderName::try_from(key)?, value.try_into()?);
            }
        }
        let (client, _) = match tls_config {
            None => {
                connect(request)?
            },
            Some(connector) => {
                let stream = TcpStream::connect(url.socket_addrs(|| None)?[0])?;
                match client_tls_with_config(request, stream, None, Some(NativeTls(connector))) {
                    Ok(websocket) => Ok(websocket),
                    Err(err) => Err(Error::InvalidHandshake(err.to_string()))
                }?
            }
        };

        Ok(WebsocketTransport {
            client: Arc::new(Mutex::new(client)),
            // SAFTEY: already a URL parsing can't fail
            base_url: Arc::new(RwLock::new(url)),
        })
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) fn upgrade(&self) -> Result<()> {
        let mut client = self.client.lock()?;

        // send the probe packet, the text `2probe` represents a ping packet with
        // the content `probe`
        client.write_message(Message::text(Cow::Borrowed(from_utf8(&Bytes::from(
            Packet::new(PacketId::Ping, Bytes::from("probe")),
        ))?)))?;

        // expect to receive a probe packet
        let message = client.read_message()?;
        let payload = message.into_data();
        if Packet::try_from(Bytes::from(payload))?
            != Packet::new(PacketId::Pong, Bytes::from("probe"))
        {
            return Err(Error::InvalidPacket());
        }

        // finally send the upgrade request. the payload `5` stands for an upgrade
        // packet without any payload
        client.write_message(Message::text(Cow::Borrowed(from_utf8(&Bytes::from(
            Packet::new(PacketId::Upgrade, Bytes::from("")),
        ))?)))?;

        Ok(())
    }
}

impl Transport for WebsocketTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        let message = if is_binary_att {
            Message::binary(Cow::Borrowed(data.as_ref()))
        } else {
            Message::text(Cow::Borrowed(std::str::from_utf8(data.as_ref())?))
        };

        let mut writer = self.client.lock()?;
        writer.write_message(message)?;
        drop(writer);

        Ok(())
    }

    fn poll(&self) -> Result<Bytes> {
        let mut receiver = self.client.lock()?;

        let message = receiver.read_message()?;
        drop(receiver);
        match message {
            Message::Binary(binary) => {
                let mut message = BytesMut::with_capacity(binary.len() + 1);
                // Prepend the message ID for consistency.
                message.put_u8(PacketId::Message as u8);
                message.put(binary.as_ref());

                Ok(message.freeze())
            }
            _ => Ok(Bytes::from(message.into_data())),
        }
    }

    fn base_url(&self) -> Result<url::Url> {
        Ok(self.base_url.read()?.clone())
    }

    fn set_base_url(&self, url: url::Url) -> Result<()> {
        let mut url = url;
        if !url
            .query_pairs()
            .any(|(k, v)| k == "transport" && v == "websocket")
        {
            url.query_pairs_mut().append_pair("transport", "websocket");
        }
        url.set_scheme("wss").unwrap();
        *self.base_url.write()? = url;
        Ok(())
    }
}

impl std::fmt::Debug for WebsocketTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "WebsocketSecureTransport(base_url: {:?})",
            self.base_url(),
        ))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::ENGINE_IO_VERSION;
    use std::str::FromStr;
    fn new() -> Result<WebsocketTransport> {
        let url = crate::test::engine_io_server_secure()?.to_string()
            + "engine.io/?EIO="
            + &ENGINE_IO_VERSION.to_string();
        WebsocketTransport::new(
            Url::from_str(&url[..])?,
            Some(crate::test::tls_connector()?),
            None,
        )
    }
    #[test]
    fn websocket_secure_transport_base_url() -> Result<()> {
        let transport = new()?;
        let mut url = crate::test::engine_io_server_secure()?;
        url.set_path("/engine.io/");
        url.query_pairs_mut()
            .append_pair("EIO", &ENGINE_IO_VERSION.to_string())
            .append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();
        assert_eq!(transport.base_url()?.to_string(), url.to_string());
        transport.set_base_url(reqwest::Url::parse("https://127.0.0.1")?)?;
        assert_eq!(
            transport.base_url()?.to_string(),
            "wss://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url()?.to_string(), url.to_string());

        transport.set_base_url(reqwest::Url::parse(
            "http://127.0.0.1/?transport=websocket",
        )?)?;
        assert_eq!(
            transport.base_url()?.to_string(),
            "wss://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url()?.to_string(), url.to_string());
        Ok(())
    }

    #[test]
    fn websocket_secure_debug() -> Result<()> {
        let transport = new()?;
        assert_eq!(
            format!("{:?}", transport),
            format!(
                "WebsocketSecureTransport(base_url: {:?})",
                transport.base_url()
            )
        );
        Ok(())
    }
}
