/// Defines the events that could be sent or received.
pub mod event;
mod packet;
/// Defines the types of payload (binary or string), that
/// could be sent or received.
pub mod payload;
pub mod socket;

#[cfg(test)]
pub(crate) mod test {
    /// The socket.io server for testing runs on port 4200
    const SERVER_URL: &str = "http://localhost:4200";
    use url::Url;

    pub(crate) fn socket_io_server() -> crate::error::Result<Url> {
        let url = std::env::var("SOCKET_IO_SERVER").unwrap_or_else(|_| SERVER_URL.to_owned());
        Ok(Url::parse(&url)?)
    }
}
