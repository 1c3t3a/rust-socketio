use std::fmt::{self, Display, Formatter};
use std::str;
use base64::DecodeError;


/// An enumeration of all possible Errors in the socket.io context.
#[derive(Debug)]
pub enum Error {
    InvalidPacketId(u8),
    EmptyPacket,
    IncompletePacket,
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
}


impl From<DecodeError> for Error {
    fn from(error: DecodeError) -> Self {
        Error::Base64Error(error)
    }
}

impl From<str::Utf8Error> for Error {
    fn from(error: str::Utf8Error) -> Self {
        Error::Utf8Error(error)
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Error::ReqwestError(error)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match &self {
            Error::InvalidPacketId(id) => write!(f, "Invalid packet id: {}", id),
            Error::EmptyPacket => write!(f, "Error while parsing an empty packet"),
            Error::IncompletePacket => write!(f, "Error while parsing an incomplete packet"),
            Error::Utf8Error(e) => {
                write!(f, "An error occured while decoding the utf-8 text: {}", e)
            }
            Error::Base64Error(e) => {
                write!(f, "An error occured while encoding/decoding base64: {}", e)
            }
            Error::InvalidUrl(url) => write!(f, "Unable to connect to: {}", url),
            Error::ReqwestError(error) => {
                write!(f, "Error during connection via Reqwest: {}", error)
            }
            Error::HandshakeError(response) => {
                write!(f, "Got illegal handshake response: {}", response)
            }
            Error::ActionBeforeOpen => {
                write!(f, "Called an action before the connection was established")
            }
            Error::HttpError(status_code) => write!(
                f,
                "Network request returned with status code: {}",
                status_code
            ),
            Error::InvalidJson(string) => write!(f, "string is not json serializable: {}", string),
            Error::DidNotReceiveProperAck(id) => write!(f, "Did not receive an ack for id: {}", id),
            Error::IllegalActionAfterOpen => write!(f, "An illegal action (such as setting a callback after being connected) was triggered")
        }
    }
}
