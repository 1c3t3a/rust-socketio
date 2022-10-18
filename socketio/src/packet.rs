use crate::error::{Error, Result};
use crate::Error::{InvalidJson, InvalidUtf8};
use bytes::{BufMut, Bytes, BytesMut};
use regex::Regex;
use std::convert::TryFrom;

/// An enumeration of the different `Packet` types in the `socket.io` protocol.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PacketId {
    Connect = 0,
    Disconnect = 1,
    Event = 2,
    Ack = 3,
    ConnectError = 4,
    BinaryEvent = 5,
    BinaryAck = 6,
}

/// A packet which gets sent or received during in the `socket.io` protocol.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Packet {
    pub packet_type: PacketId,
    pub nsp: String,
    pub data: Option<String>,
    pub id: Option<i32>,
    pub attachment_count: u8,
    pub attachments: Option<Vec<Bytes>>,
}

impl Default for Packet {
    fn default() -> Self {
        Self {
            packet_type: PacketId::Event,
            nsp: String::from("/"),
            data: None,
            id: None,
            attachment_count: 0,
            attachments: None,
        }
    }
}

impl TryFrom<u8> for PacketId {
    type Error = Error;
    fn try_from(b: u8) -> Result<Self> {
        PacketId::try_from(b as char)
    }
}

impl TryFrom<char> for PacketId {
    type Error = Error;
    fn try_from(b: char) -> Result<Self> {
        match b {
            '0' => Ok(PacketId::Connect),
            '1' => Ok(PacketId::Disconnect),
            '2' => Ok(PacketId::Event),
            '3' => Ok(PacketId::Ack),
            '4' => Ok(PacketId::ConnectError),
            '5' => Ok(PacketId::BinaryEvent),
            '6' => Ok(PacketId::BinaryAck),
            _ => Err(Error::InvalidPacketId(b)),
        }
    }
}

impl Packet {
    /// Creates an instance.
    pub const fn new(
        packet_type: PacketId,
        nsp: String,
        data: Option<String>,
        id: Option<i32>,
        attachment_count: u8,
        attachments: Option<Vec<Bytes>>,
    ) -> Self {
        Packet {
            packet_type,
            nsp,
            data,
            id,
            attachment_count,
            attachments,
        }
    }
}

impl From<Packet> for Bytes {
    fn from(packet: Packet) -> Self {
        Bytes::from(&packet)
    }
}

impl From<&Packet> for Bytes {
    /// Method for encoding from a `Packet` to a `u8` byte stream.
    /// The binary payload of a packet is not put at the end of the
    /// stream as it gets handled and send by it's own logic via the socket.
    fn from(packet: &Packet) -> Bytes {
        // first the packet type
        let mut string = (packet.packet_type as u8).to_string();

        // eventually a number of attachments, followed by '-'
        if let PacketId::BinaryAck | PacketId::BinaryEvent = packet.packet_type {
            string.push_str(&packet.attachment_count.to_string());
            string.push('-');
        }

        // if the namespace is different from the default one append it as well,
        // followed by ','
        if packet.nsp != "/" {
            string.push_str(packet.nsp.as_ref());
            string.push(',');
        }

        // if an id is present append it...
        if let Some(id) = packet.id.as_ref() {
            string.push_str(&id.to_string());
        }

        let mut buffer = BytesMut::new();
        buffer.put(string.as_ref());
        if packet.attachments.as_ref().is_some() {
            // check if an event type is present
            let placeholder = if let Some(event_type) = packet.data.as_ref() {
                format!(
                    "[{},{{\"_placeholder\":true,\"num\":{}}}]",
                    event_type,
                    packet.attachment_count - 1,
                )
            } else {
                format!(
                    "[{{\"_placeholder\":true,\"num\":{}}}]",
                    packet.attachment_count - 1,
                )
            };

            // build the buffers
            buffer.put(placeholder.as_ref());
        } else if let Some(data) = packet.data.as_ref() {
            buffer.put(data.as_ref());
        }

        buffer.freeze()
    }
}

impl TryFrom<Bytes> for Packet {
    type Error = Error;
    fn try_from(value: Bytes) -> Result<Self> {
        Packet::try_from(&value)
    }
}

impl TryFrom<&Bytes> for Packet {
    type Error = Error;
    /// Decodes a packet given a `Bytes` type.
    /// The binary payload of a packet is not put at the end of the
    /// stream as it gets handled and send by it's own logic via the socket.
    /// Therefore this method does not return the correct value for the
    /// binary data, instead the socket is responsible for handling
    /// this member. This is done because the attachment is usually
    /// send in another packet.
    fn try_from(payload: &Bytes) -> Result<Packet> {
        let mut packet: Packet = Default::default();
        let payload_utf8 =
            String::from_utf8(payload.to_vec()).map_err(|e| InvalidUtf8(e.utf8_error()))?;
        let mut utf8_iter = payload_utf8.chars().into_iter().peekable();
        let mut next_utf8;
        let mut char_buf: Vec<char> = vec![];

        // packet_type
        packet.packet_type =
            PacketId::try_from(utf8_iter.next().ok_or(Error::IncompletePacket())?)?;

        // attachment_count
        if let PacketId::BinaryAck | PacketId::BinaryEvent = packet.packet_type {
            loop {
                next_utf8 = utf8_iter.peek().ok_or(Error::IncompletePacket())?;
                if *next_utf8 == '-' {
                    let _ = utf8_iter.next(); // consume '-' char
                    break;
                }
                char_buf.push(utf8_iter.next().unwrap()); // SAFETY: already peeked
            }
        }
        let count_str: String = char_buf.iter().collect();
        if let Ok(count) = count_str.parse::<u8>() {
            packet.attachment_count = count;
        }

        // namespace
        char_buf.clear();
        next_utf8 = match utf8_iter.peek() {
            Some(c) => c,
            None => return Ok(packet),
        };

        if *next_utf8 == '/' {
            char_buf.push(utf8_iter.next().unwrap()); // SAFETY: already peeked
            loop {
                next_utf8 = utf8_iter.peek().ok_or(Error::IncompletePacket())?;
                if *next_utf8 == ',' {
                    let _ = utf8_iter.next(); // consume ','
                    break;
                }
                char_buf.push(utf8_iter.next().unwrap()); // SAFETY: already peeked
            }
        }
        if !char_buf.is_empty() {
            packet.nsp = char_buf.iter().collect();
        }

        // id
        char_buf.clear();
        next_utf8 = match utf8_iter.peek() {
            None => return Ok(packet),
            Some(c) => c,
        };

        loop {
            if !next_utf8.is_ascii_digit() {
                break;
            }
            char_buf.push(utf8_iter.next().unwrap()); // SAFETY: already peeked
            next_utf8 = match utf8_iter.peek() {
                None => return Ok(packet),
                Some(c) => c,
            };
        }

        let count_str: String = char_buf.iter().collect();
        if let Ok(count) = count_str.parse::<i32>() {
            packet.id = Some(count);
        }

        // data
        let json_str: String = utf8_iter.into_iter().collect();
        let json_data: serde_json::Value = serde_json::from_str(&json_str).map_err(InvalidJson)?;

        match packet.packet_type {
            PacketId::BinaryAck | PacketId::BinaryEvent => {
                let re_close = Regex::new(r",]$|]$").unwrap();
                let mut str = json_data
                    .to_string()
                    .replace("{\"_placeholder\":true,\"num\":0}", "");

                if str.starts_with('[') {
                    str.remove(0);
                }
                str = re_close.replace(&str, "").to_string();

                if str.is_empty() {
                    packet.data = None
                } else {
                    packet.data = Some(str)
                }
            }
            _ => packet.data = Some(json_data.to_string()),
        };

        Ok(packet)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    /// This test suite is taken from the explanation section here:
    /// https://github.com/socketio/socket.io-protocol
    fn test_decode() {
        let payload = Bytes::from_static(b"0{\"token\":\"123\"}");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                "/".to_owned(),
                Some(String::from("{\"token\":\"123\"}")),
                None,
                0,
                None,
            ),
            packet.unwrap()
        );

        let utf8_data = "{\"token™\":\"123\"}".to_owned();
        let utf8_payload = format!("0/admin™,{}", utf8_data);
        let payload = Bytes::from(utf8_payload);
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                "/admin™".to_owned(),
                Some(utf8_data),
                None,
                0,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"1/admin,");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Disconnect,
                "/admin".to_owned(),
                None,
                None,
                0,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"2[\"hello\",1]");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                "/".to_owned(),
                Some(String::from("[\"hello\",1]")),
                None,
                0,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"2/admin,456[\"project:delete\",123]");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                "/admin".to_owned(),
                Some(String::from("[\"project:delete\",123]")),
                Some(456),
                0,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"3/admin,456[]");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Ack,
                "/admin".to_owned(),
                Some(String::from("[]")),
                Some(456),
                0,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"4/admin,{\"message\":\"Not authorized\"}");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::ConnectError,
                "/admin".to_owned(),
                Some(String::from("{\"message\":\"Not authorized\"}")),
                None,
                0,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"51-[\"hello\",{\"_placeholder\":true,\"num\":0}]");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                "/".to_owned(),
                Some(String::from("\"hello\"")),
                None,
                1,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(
            b"51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]",
        );
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                "/admin".to_owned(),
                Some(String::from("\"project:delete\"")),
                Some(456),
                1,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"61-/admin,456[{\"_placeholder\":true,\"num\":0}]");
        let packet = Packet::try_from(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryAck,
                "/admin".to_owned(),
                None,
                Some(456),
                1,
                None,
            ),
            packet.unwrap()
        );
    }

    #[test]
    /// This test suites is taken from the explanation section here:
    /// https://github.com/socketio/socket.io-protocol
    fn test_encode() {
        let packet = Packet::new(
            PacketId::Connect,
            "/".to_owned(),
            Some(String::from("{\"token\":\"123\"}")),
            None,
            0,
            None,
        );

        assert_eq!(
            Bytes::from(&packet),
            "0{\"token\":\"123\"}".to_string().into_bytes()
        );

        let packet = Packet::new(
            PacketId::Connect,
            "/admin".to_owned(),
            Some(String::from("{\"token\":\"123\"}")),
            None,
            0,
            None,
        );

        assert_eq!(
            Bytes::from(&packet),
            "0/admin,{\"token\":\"123\"}".to_string().into_bytes()
        );

        let packet = Packet::new(
            PacketId::Disconnect,
            "/admin".to_owned(),
            None,
            None,
            0,
            None,
        );

        assert_eq!(Bytes::from(&packet), "1/admin,".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            "/".to_owned(),
            Some(String::from("[\"hello\",1]")),
            None,
            0,
            None,
        );

        assert_eq!(
            Bytes::from(&packet),
            "2[\"hello\",1]".to_string().into_bytes()
        );

        let packet = Packet::new(
            PacketId::Event,
            "/admin".to_owned(),
            Some(String::from("[\"project:delete\",123]")),
            Some(456),
            0,
            None,
        );

        assert_eq!(
            Bytes::from(&packet),
            "2/admin,456[\"project:delete\",123]"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::Ack,
            "/admin".to_owned(),
            Some(String::from("[]")),
            Some(456),
            0,
            None,
        );

        assert_eq!(
            Bytes::from(&packet),
            "3/admin,456[]".to_string().into_bytes()
        );

        let packet = Packet::new(
            PacketId::ConnectError,
            "/admin".to_owned(),
            Some(String::from("{\"message\":\"Not authorized\"}")),
            None,
            0,
            None,
        );

        assert_eq!(
            Bytes::from(&packet),
            "4/admin,{\"message\":\"Not authorized\"}"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::BinaryEvent,
            "/".to_owned(),
            Some(String::from("\"hello\"")),
            None,
            1,
            Some(vec![Bytes::from_static(&[1, 2, 3])]),
        );

        assert_eq!(
            Bytes::from(&packet),
            "51-[\"hello\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::BinaryEvent,
            "/admin".to_owned(),
            Some(String::from("\"project:delete\"")),
            Some(456),
            1,
            Some(vec![Bytes::from_static(&[1, 2, 3])]),
        );

        assert_eq!(
            Bytes::from(&packet),
            "51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::BinaryAck,
            "/admin".to_owned(),
            None,
            Some(456),
            1,
            Some(vec![Bytes::from_static(&[3, 2, 1])]),
        );

        assert_eq!(
            Bytes::from(&packet),
            "61-/admin,456[{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );
    }

    #[test]
    fn test_illegal_packet_id() {
        let _sut = PacketId::try_from(42).expect_err("error!");
        assert!(matches!(Error::InvalidPacketId(42 as char), _sut))
    }
}
