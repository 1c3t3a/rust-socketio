use crate::{
    asynchronous::{
        async_transports::WebsocketTransport as AsyncWebsocketTransport, transport::AsyncTransport,
    },
    error::Result,
    transport::Transport,
    Error,
};
use bytes::Bytes;
use http::HeaderMap;
use std::sync::Arc;
use tokio::runtime::Runtime;
use url::Url;

#[derive(Clone)]
pub struct WebsocketTransport {
    runtime: Arc<Runtime>,
    inner: Arc<AsyncWebsocketTransport>,
}

impl WebsocketTransport {
    /// Creates an instance of `WebsocketTransport`.
    pub fn new(base_url: Url, headers: Option<HeaderMap>) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let inner = runtime.block_on(AsyncWebsocketTransport::new(base_url, headers))?;

        Ok(WebsocketTransport {
            runtime: Arc::new(runtime),
            inner: Arc::new(inner),
        })
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) fn upgrade(&self) -> Result<()> {
        self.runtime.block_on(async { self.inner.upgrade().await })
    }
}

impl Transport for WebsocketTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        self.runtime
            .block_on(async { self.inner.emit(data, is_binary_att).await })
    }

    fn poll(&self) -> Result<Bytes> {
        self.runtime.block_on(async {
            let r = self.inner.poll_next().await;
            match r {
                Ok(b) => b.ok_or(Error::IncompletePacket()),
                Err(_) => Err(Error::IncompletePacket()),
            }
        })
    }

    fn base_url(&self) -> Result<url::Url> {
        self.runtime.block_on(async { self.inner.base_url().await })
    }

    fn set_base_url(&self, url: url::Url) -> Result<()> {
        self.runtime
            .block_on(async { self.inner.set_base_url(url).await })
    }
}

impl std::fmt::Debug for WebsocketTransport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "WebsocketTransport(base_url: {:?})",
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
        let url = crate::test::engine_io_server()?.to_string()
            + "engine.io/?EIO="
            + &ENGINE_IO_VERSION.to_string();
        WebsocketTransport::new(Url::from_str(&url[..])?, None)
    }

    #[test]
    fn websocket_transport_base_url() -> Result<()> {
        let transport = new()?;
        let mut url = crate::test::engine_io_server()?;
        url.set_path("/engine.io/");
        url.query_pairs_mut()
            .append_pair("EIO", &ENGINE_IO_VERSION.to_string())
            .append_pair("transport", "websocket");
        url.set_scheme("ws").unwrap();
        assert_eq!(transport.base_url()?.to_string(), url.to_string());
        transport.set_base_url(reqwest::Url::parse("https://127.0.0.1")?)?;
        assert_eq!(
            transport.base_url()?.to_string(),
            "ws://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url()?.to_string(), url.to_string());

        transport.set_base_url(reqwest::Url::parse(
            "http://127.0.0.1/?transport=websocket",
        )?)?;
        assert_eq!(
            transport.base_url()?.to_string(),
            "ws://127.0.0.1/?transport=websocket"
        );
        assert_ne!(transport.base_url()?.to_string(), url.to_string());
        Ok(())
    }

    #[test]
    fn websocket_secure_debug() -> Result<()> {
        let transport = new()?;
        assert_eq!(
            format!("{:?}", transport),
            format!("WebsocketTransport(base_url: {:?})", transport.base_url())
        );
        println!("{:?}", transport.poll().unwrap());
        println!("{:?}", transport.poll().unwrap());
        Ok(())
    }
}
