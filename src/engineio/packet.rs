extern crate base64;
use base64::{decode, encode, DecodeError};
use std::char;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::str;

/// Enumeration of the engineio Packet types.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum PacketId {
    Open = 0,
    Close = 1,
    Ping = 2,
    Pong = 3,
    Message = 4,
    Upgrade = 5,
    Noop = 6,
}

/// An enumeration of all possible Errors in this context.
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
}

/// A packet send in the engineio protocol.
#[derive(Debug, Clone)]
pub struct Packet {
    pub packet_id: PacketId,
    pub data: Vec<u8>,
}

// see https://en.wikipedia.org/wiki/Delimiter#ASCII_delimited_text
const SEPERATOR: char = '\x30';

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
        }
    }
}

/// Converts a byte into the corresponding packet id.
fn u8_to_packet_id(b: u8) -> Result<PacketId, Error> {
    match b as char {
        '0' => Ok(PacketId::Open),
        '1' => Ok(PacketId::Close),
        '2' => Ok(PacketId::Ping),
        '3' => Ok(PacketId::Pong),
        '4' => Ok(PacketId::Message),
        '5' => Ok(PacketId::Upgrade),
        '6' => Ok(PacketId::Noop),
        _ => Err(Error::InvalidPacketId(b)),
    }
}

impl Packet {
    /// Creates a new packet.
    pub fn new(packet_id: PacketId, data: Vec<u8>) -> Self {
        Packet { packet_id, data }
    }

    // TODO: Maybe replace the Vec<u8> by a u8 array as this might be inefficient
    fn decode_packet(bytes: Vec<u8>) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Err(Error::EmptyPacket);
        }

        let is_base64 = bytes[0] == b'b';

        // only 'messages' packets could be encoded
        let packet_id = if is_base64 {
            PacketId::Message
        } else {
            u8_to_packet_id(bytes[0])?
        };

        if bytes.len() == 1 && packet_id == PacketId::Message {
            return Err(Error::IncompletePacket);
        }

        let data: Vec<u8> = bytes.into_iter().skip(1).collect();

        Ok(Packet {
            packet_id,
            data: if is_base64 {
                decode(str::from_utf8(data.as_slice())?)?
            } else {
                data
            },
        })
    }

    fn encode_packet(self) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice((self.packet_id as u8).to_string().as_bytes());
        result.extend(self.data);
        result
    }

    fn encode_base64(self) -> Vec<u8> {
        assert_eq!(self.packet_id, PacketId::Message);

        let mut result = Vec::new();
        result.push(b'b');
        result.extend(encode(self.data).into_bytes());

        result
    }
}

pub fn decode_payload(payload: Vec<u8>) -> Result<Vec<Packet>, Error> {
    let mut vec = Vec::new();
    for packet_bytes in payload.split(|byte| (*byte as char) == SEPERATOR) {
        // this conversion might be inefficent, as the 'to_vec' method copies the elements
        vec.push(Packet::decode_packet((*packet_bytes).to_vec())?);
    }

    Ok(vec)
}

pub fn encode_payload(packets: Vec<Packet>) -> Vec<u8> {
    let mut vec = Vec::new();
    for packet in packets {
        // enforcing base64 encoding on the message packet
        match packet.packet_id {
            PacketId::Message => vec.extend(Packet::encode_base64(packet)),
            _ => vec.extend(Packet::encode_packet(packet)),
        }
        vec.push(SEPERATOR as u8);
    }

    // remove the last seperator
    let _ = vec.pop();
    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_error() {
        let err = Packet::decode_packet(Vec::new());
        assert!(err.is_err())
    }

    #[test]
    fn test_is_reflexive() {
        let data = "1Hello World".to_string().into_bytes();
        let packet = Packet::decode_packet(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Close);
        assert_eq!(packet.data, "Hello World".to_string().into_bytes());

        let data: Vec<u8> = "1Hello World".to_string().into_bytes();
        assert_eq!(Packet::encode_packet(packet), data);
    }

    #[test]
    fn test_binary_packet() {
        // SGVsbG8= is the encoded string for 'Hello'
        let data = "bSGVsbG8=".to_string().into_bytes();
        let packet = Packet::decode_packet(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Message);
        assert_eq!(packet.data, "Hello".to_string().into_bytes());

        let data = "bSGVsbG8=".to_string().into_bytes();
        assert_eq!(Packet::encode_base64(packet), data);
    }

    #[test]
    fn test_decode_payload() {
        let data = "1Hello\x301HelloWorld".to_string().into_bytes();
        let packets = decode_payload(data).unwrap();

        assert_eq!(packets[0].packet_id, PacketId::Close);
        assert_eq!(packets[0].data, ("Hello".to_string().into_bytes()));
        assert_eq!(packets[1].packet_id, PacketId::Close);
        assert_eq!(packets[1].data, ("HelloWorld".to_string().into_bytes()));

        let data = "1Hello\x301HelloWorld".to_string().into_bytes();
        assert_eq!(encode_payload(packets), data);
    }

    #[test]
    fn test_binary_payload() {
        let data = "bSGVsbG8=\x30bSGVsbG9Xb3JsZA==".to_string().into_bytes();
        let packets = decode_payload(data).unwrap();

        assert_eq!(packets[0].packet_id, PacketId::Message);
        assert_eq!(packets[0].data, ("Hello".to_string().into_bytes()));
        assert_eq!(packets[1].packet_id, PacketId::Message);
        assert_eq!(packets[1].data, ("HelloWorld".to_string().into_bytes()));

        let data = "bSGVsbG8=\x30bSGVsbG9Xb3JsZA==".to_string().into_bytes();
        assert_eq!(encode_payload(packets), data);
    }
}
