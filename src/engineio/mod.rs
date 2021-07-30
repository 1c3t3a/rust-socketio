pub mod packet;
pub mod socket;
pub mod transport;
pub mod transports;

#[cfg(test)]
pub(crate) mod test {
    /// The `engine.io` server for testing runs on port 4201
    const SERVER_URL: &str = "http://localhost:4201";
    const SERVER_URL_SECURE: &str = "https://localhost:4202";
    use url::Url;

    pub(crate) fn engine_io_server() -> crate::error::Result<Url> {
        let url = std::env::var("ENGINE_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());
        Ok(Url::parse(&url)?)
    }

    pub(crate) fn engine_io_server_secure() -> crate::error::Result<Url> {
        let url = std::env::var("ENGINE_IO_SECURE_SERVER")
            .unwrap_or_else(|_| SERVER_URL_SECURE.to_owned());
        Ok(Url::parse(&url)?)
    }
}
