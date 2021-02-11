use base64::DecodeError;
use std::str;
use std::{
    fmt::{self, Display, Formatter},
    num::ParseIntError,
};

/// Enumeration of all possible errors in the `socket.io` context.
#[derive(Debug)]
pub enum Error {
    InvalidPacketId(u8),
    EmptyPacket,
    IncompletePacket,
    InvalidPacket,
    Utf8Error(str::Utf8Error),
    Base64Error(DecodeError),
    InvalidUrl(String),
    ReqwestError(reqwest::Error),
    HttpError(u16),
    HandshakeError(String),
    ActionBeforeOpen,
    InvalidJson(String),
    DidNotReceiveProperAck(i32),
    IllegalActionAfterOpen,
    PoisonedLockError,
}

impl From<DecodeError> for Error {
    fn from(error: DecodeError) -> Self {
        Self::Base64Error(error)
    }
}

impl<T> From<std::sync::PoisonError<T>> for Error {
    fn from(_: std::sync::PoisonError<T>) -> Self {
        Self::PoisonedLockError
    }
}

impl From<str::Utf8Error> for Error {
    fn from(error: str::Utf8Error) -> Self {
        Self::Utf8Error(error)
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::ReqwestError(error)
    }
}

impl From<ParseIntError> for Error {
    fn from(_: ParseIntError) -> Self {
        // this is used for parsing integers from the a packet string
        Self::InvalidPacket
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match &self {
            Self::InvalidPacketId(id) => write!(f, "Invalid packet id: {}", id),
            Self::EmptyPacket => write!(f, "Error while parsing an empty packet"),
            Self::IncompletePacket => write!(f, "Error while parsing an incomplete packet"),
            Self::Utf8Error(e) => {
                write!(f, "An error occured while decoding the utf-8 text: {}", e)
            }
            Self::Base64Error(e) => {
                write!(f, "An error occured while encoding/decoding base64: {}", e)
            }
            Self
            ::InvalidUrl(url) => write!(f, "Unable to connect to: {}", url),
            Self::ReqwestError(error) => {
                write!(f, "Error during connection via Reqwest: {}", error)
            }
            Self::HandshakeError(response) => {
                write!(f, "Got illegal handshake response: {}", response)
            }
            Self::ActionBeforeOpen => {
                write!(f, "Called an action before the connection was established")
            }
            Self::HttpError(status_code) => write!(
                f,
                "Network request returned with status code: {}",
                status_code
            ),
            Self::InvalidJson(string) => write!(f, "string is not json serializable: {}", string),
            Self::DidNotReceiveProperAck(id) => write!(f, "Did not receive an ack for id: {}", id),
            Self::IllegalActionAfterOpen => write!(f, "An illegal action (such as setting a callback after being connected) was triggered"),
            Self::PoisonedLockError => write!(f, "A lock was poisoned"),
            Self::InvalidPacket => write!(f, "Got an invalid packet which did not follow the protocol format"),
        }
    }
}
