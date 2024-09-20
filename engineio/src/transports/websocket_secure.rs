use crate::{
    asynchronous::{
        async_transports::WebsocketSecureTransport as AsyncWebsocketSecureTransport,
        transport::AsyncTransport,
    },
    error::Result,
    transport::Transport,
    Error,
    TlsConfig,
};
use bytes::Bytes;
use http::HeaderMap;
use std::{sync::Arc, time::Duration};
use tokio::runtime::Runtime;
use url::Url;

#[derive(Clone)]
pub struct WebsocketSecureTransport {
    runtime: Arc<Runtime>,
    inner: Arc<AsyncWebsocketSecureTransport>,
}

impl WebsocketSecureTransport {
    /// Creates an instance of `WebsocketSecureTransport`.
    pub fn new(
        base_url: Url,
        tls_config: Option<TlsConfig>,
        headers: Option<HeaderMap>,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let inner = runtime.block_on(AsyncWebsocketSecureTransport::new(
            base_url, tls_config, headers,
        ))?;

        Ok(WebsocketSecureTransport {
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

impl Transport for WebsocketSecureTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        self.runtime
            .block_on(async { self.inner.emit(data, is_binary_att).await })
    }

    fn poll(&self, timeout: Duration) -> Result<Bytes> {
        self.runtime.block_on(async {
            let r = match tokio::time::timeout(timeout, self.inner.poll_next()).await {
                Ok(r) => r,
                Err(_) => return Err(Error::PingTimeout()),
            };
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

impl std::fmt::Debug for WebsocketSecureTransport {
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
    fn new() -> Result<WebsocketSecureTransport> {
        let url = crate::test::engine_io_server_secure()?.to_string()
            + "engine.io/?EIO="
            + &ENGINE_IO_VERSION.to_string();
        WebsocketSecureTransport::new(
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
