use base64::DecodeError;
use reqwest::Error as ReqwestError;
use serde_json::Error as JsonError;
use std::io::Error as IoError;
use std::num::ParseIntError;
use std::str::Utf8Error;
use thiserror::Error;
use websocket::{client::ParseError, WebSocketError};

/// Enumeration of all possible errors in the `socket.io` context.
#[derive(Error, Debug)]
#[non_exhaustive]
#[cfg_attr(tarpaulin, ignore)]
pub enum Error {
    // Conform to https://rust-lang.github.io/api-guidelines/naming.html#names-use-a-consistent-word-order-c-word-order
    // Negative verb-object
    #[error("Invalid packet id: {0}")]
    InvalidPacketId(u8),
    #[error("Error while parsing an empty packet")]
    EmptyPacket(),
    #[error("Error while parsing an incomplete packet")]
    IncompletePacket(),
    #[error("Got an invalid packet which did not follow the protocol format")]
    InvalidPacket(),
    #[error("An error occurred while decoding the utf-8 text: {0}")]
    InvalidUtf8(#[from] Utf8Error),
    #[error("An error occurred while encoding/decoding base64: {0}")]
    InvalidBase64(#[from] DecodeError),
    #[error("Invalid Url: {0}")]
    InvalidUrl(String),
    #[error("Error during connection via http: {0}")]
    IncompleteResponseFromReqwest(#[from] ReqwestError),
    #[error("Network request returned with status code: {0}")]
    IncompleteHttp(u16),
    #[error("Got illegal handshake response: {0}")]
    InvalidHandshake(String),
    #[error("Called an action before the connection was established")]
    IllegalActionBeforeOpen(),
    #[error("string is not json serializable: {0}")]
    InvalidJson(#[from] JsonError),
    #[error("Did not receive an ack for id: {0}")]
    MissingAck(i32),
    #[error("An illegal action (such as setting a callback after being connected) was triggered")]
    IllegalActionAfterOpen(),
    #[error("Specified namespace {0} is not valid")]
    IllegalNamespace(String),
    #[error("A lock was poisoned")]
    InvalidPoisonedLock(),
    #[error("Got a websocket error: {0}")]
    IncompleteResponseFromWebsocket(#[from] WebSocketError),
    #[error("Error while parsing the url for the websocket connection: {0}")]
    InvalidWebsocketURL(#[from] ParseError),
    #[error("Got an IO-Error: {0}")]
    IncompleteIo(#[from] IoError),
    #[error("The socket is closed")]
    IllegalActionAfterClose(),
    #[error("Error while parsing an integer")]
    InvalidInteger(#[from] ParseIntError),
    #[error("Missing URL")]
    MissingUrl(),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl<T> From<std::sync::PoisonError<T>> for Error {
    fn from(_: std::sync::PoisonError<T>) -> Self {
        Self::InvalidPoisonedLock()
    }
}

impl<T: Into<Error>> From<Option<T>> for Error {
    fn from(error: Option<T>) -> Self {
        error.into()
    }
}

impl From<Error> for std::io::Error {
    fn from(err: Error) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::Other, err)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use super::*;

    /// This just tests the own implementations and relies on `thiserror` for the others.
    #[test]
    fn test_error_conversion() {
        //TODO: create the errors and test all type conversions
        let mutex = Mutex::new(0);
        let _poison_error = Error::from(PoisonError::new(mutex.lock()));
        assert!(matches!(Error::InvalidPoisonedLock(), _poison_error));
    }
}
