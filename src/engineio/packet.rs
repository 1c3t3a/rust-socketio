extern crate base64;
use base64::{decode, encode};
use std::char;
use std::str;

use crate::error::Error;

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
    pub data: Vec<u8>,
}

// see https://en.wikipedia.org/wiki/Delimiter#ASCII_delimited_text
const SEPERATOR: char = '\x1e';

/// Converts a byte into the corresponding `PacketId`.
#[inline]
const fn u8_to_packet_id(b: u8) -> Result<PacketId, Error> {
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
    pub fn new(packet_id: PacketId, data: Vec<u8>) -> Self {
        Packet { packet_id, data }
    }

    // TODO: replace the Vec<u8> by a u8 array as this might be
    // inefficient

    /// Decodes a single `Packet` from an `u8` byte stream.
    fn decode_packet(bytes: Vec<u8>) -> Result<Self, Error> {
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

    /// Encodes a `Packet` into an `u8` byte stream.
    #[inline]
    fn encode_packet(self) -> Vec<u8> {
        let mut result = Vec::new();
        result.extend_from_slice((self.packet_id as u8).to_string().as_bytes());
        result.extend(self.data);
        result
    }

    // TODO: Observed some strange behavior while doing this with socket.io
    // packets, works with engine.io packets.

    /// Encodes a `Packet` with the payload as `base64`.
    #[allow(dead_code)]
    #[inline]
    fn encode_base64(self) -> Vec<u8> {
        assert_eq!(self.packet_id, PacketId::Message);

        let mut result = Vec::new();
        result.push(b'b');
        result.extend(encode(self.data).into_bytes());

        result
    }
}

/// Decodes a `payload` which in the `engine.io` context means a chain of normal
/// packets separated by a certain SEPERATOR, in this case the delimiter `\x30`.
pub fn decode_payload(payload: Vec<u8>) -> Result<Vec<Packet>, Error> {
    let mut vec = Vec::new();
    for packet_bytes in payload.split(|byte| (*byte as char) == SEPERATOR) {
        // TODO this conversion might be inefficent, as the 'to_vec' method
        // copies the elements
        vec.push(Packet::decode_packet((*packet_bytes).to_vec())?);
    }

    Ok(vec)
}

/// Encodes a payload. Payload in the `engine.io` context means a chain of
/// normal `packets` separated by a SEPERATOR, in this case the delimiter
/// `\x30`.
pub fn encode_payload(packets: Vec<Packet>) -> Vec<u8> {
    let mut vec = Vec::new();
    for packet in packets {
        // at the moment no base64 encoding is used
        vec.extend(Packet::encode_packet(packet));
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
        let data = "1Hello World".to_owned().into_bytes();
        let packet = Packet::decode_packet(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Close);
        assert_eq!(packet.data, "Hello World".to_owned().into_bytes());

        let data: Vec<u8> = "1Hello World".to_owned().into_bytes();
        assert_eq!(Packet::encode_packet(packet), data);
    }

    #[test]
    fn test_binary_packet() {
        // SGVsbG8= is the encoded string for 'Hello'
        let data = "bSGVsbG8=".to_owned().into_bytes();
        let packet = Packet::decode_packet(data).unwrap();

        assert_eq!(packet.packet_id, PacketId::Message);
        assert_eq!(packet.data, "Hello".to_owned().into_bytes());

        let data = "bSGVsbG8=".to_owned().into_bytes();
        assert_eq!(Packet::encode_base64(packet), data);
    }

    #[test]
    fn test_decode_payload() {
        let data = "1Hello\x1e1HelloWorld".to_owned().into_bytes();
        let packets = decode_payload(data).unwrap();

        assert_eq!(packets[0].packet_id, PacketId::Close);
        assert_eq!(packets[0].data, ("Hello".to_owned().into_bytes()));
        assert_eq!(packets[1].packet_id, PacketId::Close);
        assert_eq!(packets[1].data, ("HelloWorld".to_owned().into_bytes()));

        let data = "1Hello\x1e1HelloWorld".to_owned().into_bytes();
        assert_eq!(encode_payload(packets), data);
    }

    #[test]
    fn test_binary_payload() {
        let data = "bSGVsbG8=\x1ebSGVsbG9Xb3JsZA==".to_string().into_bytes();
        let packets = decode_payload(data).unwrap();

        assert_eq!(packets[0].packet_id, PacketId::Message);
        assert_eq!(packets[0].data, ("Hello".to_owned().into_bytes()));
        assert_eq!(packets[1].packet_id, PacketId::Message);
        assert_eq!(packets[1].data, ("HelloWorld".to_owned().into_bytes()));

        let data = "4Hello\x1e4HelloWorld".to_owned().into_bytes();
        assert_eq!(encode_payload(packets), data);
    }
}
