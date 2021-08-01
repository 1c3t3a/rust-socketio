use crate::error::{Error, Result};
use byte::{ctx::Str, BytesExt};
use bytes::{BufMut, Bytes, BytesMut};
use regex::Regex;

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
    pub binary_data: Option<Bytes>,
    pub id: Option<i32>,
    pub attachments: Option<u8>,
}

/// Converts a `u8` into a `PacketId`.
#[inline]
pub const fn u8_to_packet_id(b: u8) -> Result<PacketId> {
    match b as char {
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

impl Packet {
    /// Creates an instance.
    pub const fn new(
        packet_type: PacketId,
        nsp: String,
        data: Option<String>,
        binary_data: Option<Bytes>,
        id: Option<i32>,
        attachments: Option<u8>,
    ) -> Self {
        Packet {
            packet_type,
            nsp,
            data,
            binary_data,
            id,
            attachments,
        }
    }

    /// Method for encoding from a `Packet` to a `u8` byte stream.
    /// The binary payload of a packet is not put at the end of the
    /// stream as it gets handled and send by it's own logic via the socket.
    pub fn encode(&self) -> Bytes {
        // first the packet type
        let mut string = (self.packet_type as u8).to_string();

        // eventually a number of attachments, followed by '-'
        if let PacketId::BinaryAck | PacketId::BinaryEvent = self.packet_type {
            string.push_str(&self.attachments.as_ref().unwrap().to_string());
            string.push('-');
        }

        // if the namespace is different from the default one append it as well,
        // followed by ','
        if self.nsp != "/" {
            string.push_str(self.nsp.as_ref());
            string.push(',');
        }

        // if an id is present append it...
        if let Some(id) = self.id.as_ref() {
            string.push_str(&id.to_string());
        }

        let mut buffer = BytesMut::new();
        buffer.put(string.as_ref());
        if self.binary_data.as_ref().is_some() {
            // check if an event type is present
            let placeholder = if let Some(event_type) = self.data.as_ref() {
                format!(
                    "[{},{{\"_placeholder\":true,\"num\":{}}}]",
                    event_type,
                    self.attachments.unwrap() - 1,
                )
            } else {
                format!(
                    "[{{\"_placeholder\":true,\"num\":{}}}]",
                    self.attachments.unwrap() - 1,
                )
            };

            // build the buffers
            buffer.put(placeholder.as_ref());
        } else if let Some(data) = self.data.as_ref() {
            buffer.put(data.as_ref());
        }

        buffer.freeze()
    }

    /// Decodes a packet given a `Bytes` type.
    /// The binary payload of a packet is not put at the end of the
    /// stream as it gets handled and send by it's own logic via the socket.
    /// Therefore this method does not return the correct value for the
    /// binary data, instead the socket is responsible for handling
    /// this member. This is done because the attachment is usually
    /// send in another packet.
    pub fn decode_bytes<'a>(payload: &'a Bytes) -> Result<Self> {
        let mut i = 0;
        let packet_id = u8_to_packet_id(*payload.first().ok_or(Error::EmptyPacket())?)?;

        let attachments = if let PacketId::BinaryAck | PacketId::BinaryEvent = packet_id {
            let start = i + 1;

            while payload.get(i).ok_or(Error::IncompletePacket())? != &b'-' && i < payload.len() {
                i += 1;
            }
            Some(
                payload
                    .iter()
                    .skip(start)
                    .take(i - start)
                    .map(|byte| *byte as char)
                    .collect::<String>()
                    .parse::<u8>()?,
            )
        } else {
            None
        };

        let nsp: &'a str = if payload.get(i + 1).ok_or(Error::IncompletePacket())? == &b'/' {
            let mut start = i + 1;
            while payload.get(i).ok_or(Error::IncompletePacket())? != &b',' && i < payload.len() {
                i += 1;
            }
            let len = i - start;
            payload
                .read_with(&mut start, Str::Len(len))
                .map_err(|_| Error::IncompletePacket())?
        } else {
            "/"
        };

        let next = payload.get(i + 1).unwrap_or(&b'_');
        let id = if (*next as char).is_digit(10) && i < payload.len() {
            let start = i + 1;
            i += 1;
            while (*payload.get(i).ok_or(Error::IncompletePacket())? as char).is_digit(10)
                && i < payload.len()
            {
                i += 1;
            }

            Some(
                payload
                    .iter()
                    .skip(start)
                    .take(i - start)
                    .map(|byte| *byte as char)
                    .collect::<String>()
                    .parse::<i32>()?,
            )
        } else {
            None
        };

        let data = if payload.get(i + 1).is_some() {
            let start = if id.is_some() { i } else { i + 1 };

            let mut json_data = serde_json::Value::Null;

            let mut end = payload.len();
            while serde_json::from_str::<serde_json::Value>(
                &payload
                    .iter()
                    .skip(start)
                    .take(end - start)
                    .map(|byte| *byte as char)
                    .collect::<String>(),
            )
            .is_err()
            {
                end -= 1;
            }

            if end != start {
                // unwrapping here is infact safe as we checked for errors in the
                // condition of the loop
                json_data = serde_json::from_str(
                    &payload
                        .iter()
                        .skip(start)
                        .take(end - start)
                        .map(|byte| *byte as char)
                        .collect::<String>(),
                )
                .unwrap();
            }

            match packet_id {
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
                        None
                    } else {
                        Some(str)
                    }
                }
                _ => Some(json_data.to_string()),
            }
        } else {
            None
        };

        Ok(Packet::new(
            packet_id,
            nsp.to_owned(),
            data,
            None,
            id,
            attachments,
        ))
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
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                "/".to_owned(),
                Some(String::from("{\"token\":\"123\"}")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"0/admin,{\"token\":\"123\"}");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                "/admin".to_owned(),
                Some(String::from("{\"token\":\"123\"}")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"1/admin,");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Disconnect,
                "/admin".to_owned(),
                None,
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"2[\"hello\",1]");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                "/".to_owned(),
                Some(String::from("[\"hello\",1]")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"2/admin,456[\"project:delete\",123]");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                "/admin".to_owned(),
                Some(String::from("[\"project:delete\",123]")),
                None,
                Some(456),
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"3/admin,456[]");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Ack,
                "/admin".to_owned(),
                Some(String::from("[]")),
                None,
                Some(456),
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"4/admin,{\"message\":\"Not authorized\"}");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::ConnectError,
                "/admin".to_owned(),
                Some(String::from("{\"message\":\"Not authorized\"}")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"51-[\"hello\",{\"_placeholder\":true,\"num\":0}]");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                "/".to_owned(),
                Some(String::from("\"hello\"")),
                None,
                None,
                Some(1),
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(
            b"51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]",
        );
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                "/admin".to_owned(),
                Some(String::from("\"project:delete\"")),
                None,
                Some(456),
                Some(1),
            ),
            packet.unwrap()
        );

        let payload = Bytes::from_static(b"61-/admin,456[{\"_placeholder\":true,\"num\":0}]");
        let packet = Packet::decode_bytes(&payload);
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryAck,
                "/admin".to_owned(),
                None,
                None,
                Some(456),
                Some(1),
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
            None,
            None,
        );

        assert_eq!(
            packet.encode(),
            "0{\"token\":\"123\"}".to_string().into_bytes()
        );

        let packet = Packet::new(
            PacketId::Connect,
            "/admin".to_owned(),
            Some(String::from("{\"token\":\"123\"}")),
            None,
            None,
            None,
        );

        assert_eq!(
            packet.encode(),
            "0/admin,{\"token\":\"123\"}".to_string().into_bytes()
        );

        let packet = Packet::new(
            PacketId::Disconnect,
            "/admin".to_owned(),
            None,
            None,
            None,
            None,
        );

        assert_eq!(packet.encode(), "1/admin,".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            "/".to_owned(),
            Some(String::from("[\"hello\",1]")),
            None,
            None,
            None,
        );

        assert_eq!(packet.encode(), "2[\"hello\",1]".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            "/admin".to_owned(),
            Some(String::from("[\"project:delete\",123]")),
            None,
            Some(456),
            None,
        );

        assert_eq!(
            packet.encode(),
            "2/admin,456[\"project:delete\",123]"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::Ack,
            "/admin".to_owned(),
            Some(String::from("[]")),
            None,
            Some(456),
            None,
        );

        assert_eq!(packet.encode(), "3/admin,456[]".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::ConnectError,
            "/admin".to_owned(),
            Some(String::from("{\"message\":\"Not authorized\"}")),
            None,
            None,
            None,
        );

        assert_eq!(
            packet.encode(),
            "4/admin,{\"message\":\"Not authorized\"}"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::BinaryEvent,
            "/".to_owned(),
            Some(String::from("\"hello\"")),
            Some(Bytes::from_static(&[1, 2, 3])),
            None,
            Some(1),
        );

        assert_eq!(
            packet.encode(),
            "51-[\"hello\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::BinaryEvent,
            "/admin".to_owned(),
            Some(String::from("\"project:delete\"")),
            Some(Bytes::from_static(&[1, 2, 3])),
            Some(456),
            Some(1),
        );

        assert_eq!(
            packet.encode(),
            "51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let packet = Packet::new(
            PacketId::BinaryAck,
            "/admin".to_owned(),
            None,
            Some(Bytes::from_static(&[3, 2, 1])),
            Some(456),
            Some(1),
        );

        assert_eq!(
            packet.encode(),
            "61-/admin,456[{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );
    }

    #[test]
    fn test_illegal_packet_id() {
        let _sut = u8_to_packet_id(42).expect_err("error!");
        assert!(matches!(Error::InvalidPacketId(42), _sut))
    }
}
