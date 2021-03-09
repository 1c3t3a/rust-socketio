use base64::DecodeError;
use std::{num::ParseIntError, str};
use thiserror::Error;
use websocket::{client::ParseError, WebSocketError};

/// Enumeration of all possible errors in the `socket.io` context.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum Error {
    #[error("Invalid packet id: {0}")]
    InvalidPacketId(u8),
    #[error("Error while parsing an empty packet")]
    EmptyPacket,
    #[error("Error while parsing an incomplete packet")]
    IncompletePacket,
    #[error("Got an invalid packet which did not follow the protocol format")]
    InvalidPacket,
    #[error("An error occured while decoding the utf-8 text: {0}")]
    Utf8Error(#[from] str::Utf8Error),
    #[error("An error occured while encoding/decoding base64: {0}")]
    Base64Error(#[from] DecodeError),
    #[error("Invalid Url: {0}")]
    InvalidUrl(String),
    #[error("Error during connection via http: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("Network request returned with status code: {0}")]
    HttpError(u16),
    #[error("Got illegal handshake response: {0}")]
    HandshakeError(String),
    #[error("Called an action before the connection was established")]
    ActionBeforeOpen,
    #[error("string is not json serializable: {0}")]
    InvalidJson(String),
    #[error("Did not receive an ack for id: {0}")]
    DidNotReceiveProperAck(i32),
    #[error("An illegal action (such as setting a callback after being connected) was triggered")]
    IllegalActionAfterOpen,
    #[error("Specified namespace {0} is not valid")]
    IllegalNamespace(String),
    #[error("A lock was poisoned")]
    PoisonedLockError,
    #[error("Got a websocket error: {0}")]
    FromWebsocketError(#[from] WebSocketError),
    #[error("Error while parsing the url for the websocket connection: {0}")]
    FromWebsocketParseError(#[from] ParseError),
    #[error("Got an IO-Error: {0}")]
    FromIoError(#[from] std::io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

impl<T> From<std::sync::PoisonError<T>> for Error {
    fn from(_: std::sync::PoisonError<T>) -> Self {
        Self::PoisonedLockError
    }
}

impl From<ParseIntError> for Error {
    fn from(_: ParseIntError) -> Self {
        // this is used for parsing integers from the a packet string
        Self::InvalidPacket
    }
}
