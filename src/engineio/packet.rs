extern crate base64;
use std::ops::Index;
use base64::{decode, encode};
use bytes::{BufMut, Bytes, BytesMut};
use serde::{Deserialize, Serialize};
use std::char;
use std::convert::TryInto;
use std::convert::TryFrom;

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
    Base64,
}

impl From<PacketId> for String {
    fn from(packet: PacketId) -> Self {
        match packet {
            PacketId::Base64 => "b".to_owned(),
            _ => (packet as u8).to_string(),
        }
    }
}

impl TryFrom<u8> for PacketId {
    type Error = Error;
    /// Converts a byte into the corresponding `PacketId`.
    fn try_from(b: u8) -> Result<PacketId> {
        
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
        Ok(serde_json::from_slice(self.data[..].as_ref())?)
    }
}

/// A `Packet` sent via the `engine.io` protocol.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub packet_id: PacketId,
    pub data: Bytes,
}

impl Packet {
    /// Creates a new `Packet`.
    pub fn new(packet_id: PacketId, data: Bytes) -> Self {
        Packet {
            packet_id,
            data,
        }
    }
}

impl TryFrom<Bytes> for Packet {
    type Error = Error;
    /// Decodes a single `Packet` from an `u8` byte stream.
    fn try_from(
        bytes: Bytes,
    ) -> std::result::Result<Self, <Self as std::convert::TryFrom<Bytes>>::Error> {
        if bytes.is_empty() {
            return Err(Error::IncompletePacket());
        }

        let is_base64 = *bytes.get(0).ok_or(Error::IncompletePacket())? == b'b';

        // only 'messages' packets could be encoded
        let packet_id = if is_base64 {
            PacketId::Message
        } else {
            (*bytes.get(0).ok_or(Error::IncompletePacket())? as u8).try_into()?
        };

        if bytes.len() == 1 && packet_id == PacketId::Message {
            return Err(Error::IncompletePacket());
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
}

impl From<Packet> for Bytes {
    /// Encodes a `Packet` into an `u8` byte stream.
    fn from(packet: Packet) -> Self {
        let mut result = BytesMut::with_capacity(packet.data.len() + 1);
        result.put(String::from(packet.packet_id).as_bytes());
        if packet.packet_id == PacketId::Base64 {
            result.extend(encode(packet.data).into_bytes());
        } else {
            result.put(packet.data);
        }
        result.freeze()
    }
}

pub struct Payload(Vec<Packet>);

impl Payload {
    // see https://en.wikipedia.org/wiki/Delimiter#ASCII_delimited_text
    const SEPARATOR: char = '\x1e';

    pub fn new(packets: Vec<Packet>) -> Self {
        Payload(packets)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl TryFrom<Bytes> for Payload {
    type Error = Error;
    /// Decodes a `payload` which in the `engine.io` context means a chain of normal
    /// packets separated by a certain SEPARATOR, in this case the delimiter `\x30`.
    fn try_from(payload: Bytes) -> Result<Self> {
        let mut vec = Vec::new();
        let mut last_index = 0;

        for i in 0..payload.len() {
            if *payload.get(i).unwrap() as char == Self::SEPARATOR {
                vec.push(Packet::try_from(payload.slice(last_index..i))?);
                last_index = i + 1;
            }
        }
        // push the last packet as well
        vec.push(Packet::try_from(payload.slice(last_index..payload.len()))?);

        Ok(Payload(vec))
    }
}

impl TryFrom<Payload> for Bytes {
    type Error = Error;
    /// Encodes a payload. Payload in the `engine.io` context means a chain of
    /// normal `packets` separated by a SEPARATOR, in this case the delimiter
    /// `\x30`.
    fn try_from(packets: Payload) -> Result<Self> {
        let mut buf = BytesMut::new();
        for packet in packets {
            // at the moment no base64 encoding is used
            buf.extend(Bytes::from(packet.clone()));
            buf.put_u8(Payload::SEPARATOR as u8);
        }

        // remove the last separator
        let _ = buf.split_off(buf.len() - 1);
        Ok(buf.freeze())
    }
}

// This exposes a vec specific type (IntoIter)
impl IntoIterator for Payload {
    type Item = Packet;
    type IntoIter = std::vec::IntoIter<Self::Item>;
    fn into_iter(self) -> <Self as std::iter::IntoIterator>::IntoIter {
        self.0.into_iter()
    }
}

impl Index<usize> for Payload {
    type Output = Packet;
    fn index(&self, index: usize) -> &Packet {
        &self.0[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_error() {
        let err = Packet::try_from(BytesMut::with_capacity(10).freeze());
        assert!(err.is_err())
    }

    #[test]
    fn test_is_reflexive() {
        let data = Bytes::from_static(b"1Hello World");
        let packet = Packet::try_from(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Close);
        assert_eq!(packet.data, Bytes::from_static(b"Hello World"));

        let data = Bytes::from_static(b"1Hello World");
        assert_eq!(Bytes::from(packet), data);
    }

    #[test]
    fn test_binary_packet() {
        // SGVsbG8= is the encoded string for 'Hello'
        let data = Bytes::from_static(b"bSGVsbG8=");
        let packet = Packet::try_from(data.clone()).unwrap();

        assert_eq!(packet.packet_id, PacketId::Message);
        assert_eq!(packet.data, Bytes::from_static(b"Hello"));

        assert_eq!(Bytes::from(packet), data);
    }

    #[test]
    fn test_decode_payload() -> Result<()> {
        let data = Bytes::from_static(b"1Hello\x1e1HelloWorld");
        let packets = Payload::try_from(data)?;

        assert_eq!(packets[0].packet_id, PacketId::Close);
        assert_eq!(packets[0].data, Bytes::from_static(b"Hello"));
        assert_eq!(packets[1].packet_id, PacketId::Close);
        assert_eq!(packets[1].data, Bytes::from_static(b"HelloWorld"));

        let data = "1Hello\x1e1HelloWorld".to_owned().into_bytes();
        assert_eq!(Bytes::try_from(packets).unwrap(), data);

        Ok(())
    }

    #[test]
    fn test_binary_payload() {
        let data = Bytes::from_static(b"bSGVsbG8=\x1ebSGVsbG9Xb3JsZA==\x1ebSGVsbG8=");
        let packets = Payload::try_from(data).unwrap();

        assert!(packets.len() == 3);
        assert_eq!(packets[0].packet_id, PacketId::Message);
        assert_eq!(packets[0].data, Bytes::from_static(b"Hello"));
        assert_eq!(packets[1].packet_id, PacketId::Message);
        assert_eq!(packets[1].data, Bytes::from_static(b"HelloWorld"));
        assert_eq!(packets[2].packet_id, PacketId::Message);
        assert_eq!(packets[2].data, Bytes::from_static(b"Hello"));

        let data = Bytes::from_static(b"4Hello\x1e4HelloWorld\x1e4Hello");
        assert_eq!(Bytes::try_from(packets).unwrap(), data);
    }

    #[test]
    fn test_packet_id_conversion_and_incompl_packet() {
        let sut = Packet::try_from(Bytes::from_static(b"4"));
        assert!(sut.is_err());
        let _sut = sut.unwrap_err();
        assert!(matches!(Error::IncompletePacket, _sut));

        let sut = PacketId::try_from(b'0');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Open);

        let sut = PacketId::try_from(b'1');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Close);

        let sut = PacketId::try_from(b'2');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Ping);

        let sut = PacketId::try_from(b'3');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Pong);

        let sut = PacketId::try_from(b'4');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Message);

        let sut = PacketId::try_from(b'5');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Upgrade);

        let sut = PacketId::try_from(b'6');
        assert!(sut.is_ok());
        assert_eq!(sut.unwrap(), PacketId::Noop);

        let sut = PacketId::try_from(42);
        assert!(sut.is_err());
        assert!(matches!(sut.unwrap_err(), Error::InvalidPacketId(42)));
    }
}
