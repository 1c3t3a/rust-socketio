extern crate base64;
use base64::{decode, encode};
use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use std::char;
use std::convert::TryInto;

use crate::error::{Error, Result};
/// Enumeration of the `engine.io` `Packet` types.
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

/// A `Packet` sent via the `engine.io` protocol.
#[derive(Debug, Clone)]
pub struct Packet {
    pub packet_id: PacketId,
    pub data: Bytes,
}

// see https://en.wikipedia.org/wiki/Delimiter#ASCII_delimited_text
const SEPARATOR: char = '\x1e';

/// Converts a byte into the corresponding `PacketId`.
#[inline]
const fn u8_to_packet_id(b: u8) -> Result<PacketId> {
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
    /// Creates a new `Packet`.
    pub fn new(packet_id: PacketId, data: Bytes) -> Self {
        let bytes = data;
        Packet {
            packet_id,
            data: bytes,
        }
    }

    /// Decodes a single `Packet` from an `u8` byte stream.
    pub(super) fn decode(bytes: Bytes) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::EmptyPacket);
        }

        let is_base64 = *bytes.get(0).ok_or(Error::IncompletePacket)? == b'b';

        // only 'messages' packets could be encoded
        let packet_id = if is_base64 {
            PacketId::Message
        } else {
            u8_to_packet_id(*bytes.get(0).ok_or(Error::IncompletePacket)?)?
        };

        if bytes.len() == 1 && packet_id == PacketId::Message {
            return Err(Error::IncompletePacket);
        }

        let data: Bytes = bytes.slice(1..);

        Ok(Packet {
            packet_id,
            data: if is_base64 {
                Bytes::from(decode(data.as_ref())?)
            } else {
                data
            },
        })
    }

    /// Encodes a `Packet` into an `u8` byte stream.
    #[inline]
    pub(super) fn encode(self) -> Bytes {
        let mut result = BytesMut::with_capacity(self.data.len() + 1);
        result.put((self.packet_id as u8).to_string().as_bytes());
        result.put(self.data);
        result.freeze()
    }

    // Observed some strange behavior while doing this with socket.io
    // packets, works with engine.io packets.
    /// Encodes a `Packet` with the payload as `base64`.
    #[allow(dead_code)]
    #[inline]
    pub(super) fn encode_base64(self) -> Bytes {
        assert_eq!(self.packet_id, PacketId::Message);

        let mut result = BytesMut::with_capacity(self.data.len() + 1);
        result.put_u8(b'b');
        result.extend(encode(self.data).into_bytes());

        result.freeze()
    }
}

/// Decodes a `payload` which in the `engine.io` context means a chain of normal
/// packets separated by a certain SEPARATOR, in this case the delimiter `\x30`.
pub fn decode_payload(payload: Bytes) -> Result<Vec<Packet>> {
    let mut vec = Vec::new();
    let mut last_index = 0;

    for i in 0..payload.len() {
        if *payload.get(i).unwrap() as char == SEPARATOR {
            vec.push(Packet::decode(payload.slice(last_index..i))?);
            last_index = i + 1;
        }
    }
    // push the last packet as well
    vec.push(Packet::decode(payload.slice(last_index..payload.len()))?);

    Ok(vec)
}

/// Encodes a payload. Payload in the `engine.io` context means a chain of
/// normal `packets` separated by a SEPARATOR, in this case the delimiter
/// `\x30`.
pub fn encode_payload(packets: Vec<Packet>) -> Bytes {
    let mut buf = BytesMut::new();
    for packet in packets {
        // at the moment no base64 encoding is used
        buf.extend(Packet::encode(packet));
        buf.put_u8(SEPARATOR as u8);
    }

    // remove the last separator
    let _ = buf.split_off(buf.len() - 1);
    buf.freeze()
}

/// Data which gets exchanged in a handshake as defined by the server.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct HandshakePacket {
    pub sid: String,
    pub upgrades: Vec<String>,
    #[serde(rename = "pingInterval")]
    pub ping_interval: u64,
    #[serde(rename = "pingTimeout")]
    pub ping_timeout: u64,
}

impl TryInto<HandshakePacket> for Packet {
    type Error = Error;
    fn try_into(self) -> Result<HandshakePacket> {
        // TODO: properly cast serde_json to JsonError
        if let Ok(handshake) = serde_json::from_slice::<HandshakePacket>(self.data[..].as_ref()) {
            Ok(handshake)
        } else {
            Err(Error::JsonError)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_error() {
        let err = Packet::decode(BytesMut::with_capacity(10).freeze());
        assert!(err.is_err())
    }

    #[test]
    fn test_is_reflexive() {
        let data = Bytes::from_static(b"1Hello World");
        let packet = Packet::decode(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Close);
        assert_eq!(packet.data, Bytes::from_static(b"Hello World"));

        let data = Bytes::from_static(b"1Hello World");
        assert_eq!(Packet::encode(packet), data);
    }

    #[test]
    fn test_binary_packet() {
        // SGVsbG8= is the encoded string for 'Hello'
        let data = Bytes::from_static(b"bSGVsbG8=");
        let packet = Packet::decode(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Message);
        assert_eq!(packet.data, Bytes::from_static(b"Hello"));

        let data = Bytes::from_static(b"bSGVsbG8=");
        assert_eq!(Packet::encode_base64(packet), data);
    }

    #[test]
    fn test_decode_payload() {
        let data = Bytes::from_static(b"1Hello\x1e1HelloWorld");
        let packets = decode_payload(data).unwrap();

        assert_eq!(packets[0].packet_id, PacketId::Close);
        assert_eq!(packets[0].data, Bytes::from_static(b"Hello"));
        assert_eq!(packets[1].packet_id, PacketId::Close);
        assert_eq!(packets[1].data, Bytes::from_static(b"HelloWorld"));

        let data = "1Hello\x1e1HelloWorld".to_owned().into_bytes();
        assert_eq!(encode_payload(packets), data);
    }

    #[test]
    fn test_binary_payload() {
        let data = Bytes::from_static(b"bSGVsbG8=\x1ebSGVsbG9Xb3JsZA==\x1ebSGVsbG8=");
        let packets = decode_payload(data).unwrap();

        assert!(packets.len() == 3);
        assert_eq!(packets[0].packet_id, PacketId::Message);
        assert_eq!(packets[0].data, Bytes::from_static(b"Hello"));
        assert_eq!(packets[1].packet_id, PacketId::Message);
        assert_eq!(packets[1].data, Bytes::from_static(b"HelloWorld"));
        assert_eq!(packets[2].packet_id, PacketId::Message);
        assert_eq!(packets[2].data, Bytes::from_static(b"Hello"));

        let data = Bytes::from_static(b"4Hello\x1e4HelloWorld\x1e4Hello");
        assert_eq!(encode_payload(packets), data);
    }

    #[test]
    fn test_packet_id_conversion_and_incompl_packet() {
        let sut = Packet::decode(Bytes::from_static(b"4"));
        assert!(sut.is_err());
        let _sut = sut.unwrap_err();
        assert!(matches!(Error::IncompletePacket, _sut));

        let sut = u8_to_packet_id(b'0');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Open);

        let sut = u8_to_packet_id(b'1');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Close);

        let sut = u8_to_packet_id(b'2');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Ping);

        let sut = u8_to_packet_id(b'3');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Pong);

        let sut = u8_to_packet_id(b'4');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Message);

        let sut = u8_to_packet_id(b'5');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Upgrade);

        let sut = u8_to_packet_id(b'6');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Noop);

        let sut = u8_to_packet_id(42);
        assert!(sut.is_err());
        assert!(matches!(sut.unwrap_err(), Error::InvalidPacketId(42)));
    }
}
