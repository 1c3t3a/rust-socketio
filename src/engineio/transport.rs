use crate::error::Result;
use adler32::adler32;
use bytes::{Buf, Bytes};
use std::fmt::Debug;
use std::fmt::Formatter;
use std::io::Read;
use std::io::Write;
use std::time::SystemTime;
use url::Url;

pub trait Transport: Clone {
    /// Sends a packet to the server. This optionally handles sending of a
    /// socketio binary attachment via the boolean attribute `is_binary_att`.
    fn emit(&self, data: Bytes, is_binary_att: bool) -> Result<()>;

    /// Performs the server long polling procedure as long as the client is
    /// connected. This should run separately at all time to ensure proper
    /// response handling from the server.
    fn poll(&self) -> Result<Bytes>;

    /// Returns start of the url. ex. http://localhost:2998/engine.io/?EIO=4&transport=polling
    /// Must have EIO and transport already set.
    // TODO: Add a URL type
    fn base_url(&self) -> Result<Url>;

    /// Used to update the base path, like when adding the sid.
    fn set_base_url(&self, base_url: Url) -> Result<()>;

    /// Full query address
    fn address(&self) -> Result<Url> {
        let reader = format!("{:#?}", SystemTime::now());
        let hash = adler32(reader.as_bytes()).unwrap();
        let mut url = self.base_url()?;
        url.query_pairs_mut().append_pair("t", &hash.to_string());
        Ok(url)
    }
}

impl<T: Transport> Read for T {
    fn read(&mut self, buf: &mut [u8]) -> std::result::Result<usize, std::io::Error> {
        let mut bytes = self.poll()?;
        bytes.copy_to_slice(buf);
        Ok(bytes.len())
    }
}

impl<T: Transport> Write for T {
    fn write(&mut self, buf: &[u8]) -> std::result::Result<usize, std::io::Error> {
        let bytes = Bytes::copy_from_slice(buf);
        self.emit(bytes.clone(), true).unwrap();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::result::Result<(), std::io::Error> {
        // We are always flushed.
        Ok(())
    }
}

impl<T: Transport> Debug for T {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!(
            "Transport(base_url: {:?}, address: {:?})",
            self.base_url(),
            self.address()
        ))
    }
}
