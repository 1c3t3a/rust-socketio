use crate::async_transports::transport::AsyncTransport;
use crate::async_transports::websocket_secure::AsyncWebsocketSecureTransport;
use crate::error::Result;
use crate::header::HeaderMap;
use crate::transport::Transport;
use bytes::Bytes;
use native_tls::TlsConnector;
use std::sync::Arc;
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
        tls_config: Option<TlsConnector>,
        headers: Option<HeaderMap>,
    ) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;

        let mut url = base_url;
        url.query_pairs_mut().append_pair("transport", "websocket");
        url.set_scheme("wss").unwrap();

        let mut req = http::Request::builder().uri(url.clone().as_str());
        if let Some(map) = headers {
            // SAFETY: this unwrap never panics as the underlying request is just initialized and in proper state
            req.headers_mut()
                .unwrap()
                .extend::<reqwest::header::HeaderMap>(map.try_into()?);
        }

        let inner = runtime.block_on(AsyncWebsocketSecureTransport::new(
            req.body(())?,
            url,
            tls_config,
        ))?;

        Ok(WebsocketSecureTransport {
            runtime: Arc::new(runtime),
            inner: Arc::new(inner),
        })
    }

    /// Sends probe packet to ensure connection is valid, then sends upgrade
    /// request
    pub(crate) fn upgrade(&self) -> Result<()> {
        self.runtime.block_on(self.inner.upgrade())
    }
}

impl Transport for WebsocketSecureTransport {
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()> {
        self.runtime.block_on(self.inner.emit(data, is_binary_att))
    }

    fn poll(&self) -> Result<Bytes> {
        self.runtime.block_on(self.inner.poll())
    }

    fn base_url(&self) -> Result<url::Url> {
        self.runtime.block_on(self.inner.base_url())
    }

    fn set_base_url(&self, url: url::Url) -> Result<()> {
        self.runtime.block_on(self.inner.set_base_url(url))
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
