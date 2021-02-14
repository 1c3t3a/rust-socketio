use crate::error::Error;
use regex::Regex;

/// An enumeration of the different Paccket types in the socket.io protocol.
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

/// A packet which gets send or received during in the socket-io protocol.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Packet {
    pub packet_type: PacketId,
    pub nsp: String,
    pub data: Option<String>,
    pub binary_data: Option<Vec<u8>>,
    pub id: Option<i32>,
    pub attachements: Option<u8>,
}

/// Converts an u8 byte to a `PacketId`.
#[inline]
pub const fn u8_to_packet_id(b: u8) -> Result<PacketId, Error> {
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
        binary_data: Option<Vec<u8>>,
        id: Option<i32>,
        attachements: Option<u8>,
    ) -> Self {
        Packet {
            packet_type,
            nsp,
            data,
            binary_data,
            id,
            attachements,
        }
    }

    /// Encodes the packet to a u8 byte stream.
    pub fn encode(&self) -> Vec<u8> {
        // first the packet type
        let mut string = (self.packet_type as u8).to_string();

        // eventually a number of attachements, followed by '-'
        match self.packet_type {
            PacketId::BinaryAck | PacketId::BinaryEvent => {
                string.push_str(&self.attachements.as_ref().unwrap().to_string());
                string.push('-');
            }
            _ => (),
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

        let mut buffer = string.into_bytes();
        if let Some(bin_data) = self.binary_data.as_ref() {
            // check if an event type is present
            let placeholder = if let Some(event_type) = self.data.as_ref() {
                format!(
                    "[{},{{\"_placeholder\":true,\"num\":{}}}]",
                    event_type,
                    self.attachements.unwrap() - 1,
                )
            } else {
                format!(
                    "[{{\"_placeholder\":true,\"num\":{}}}]",
                    self.attachements.unwrap() - 1,
                )
            };

            // build the buffers
            buffer.extend(placeholder.into_bytes());
            buffer.extend(bin_data);
        } else if let Some(data) = self.data.as_ref() {
            buffer.extend(data.to_string().into_bytes());
        }

        buffer
    }

    /// Decodes a packet given a `Vec<u8>`.
    pub fn decode_bytes(payload: Vec<u8>) -> Result<Self, Error> {
        let mut i = 0;
        let packet_id = u8_to_packet_id(*payload.first().ok_or(Error::EmptyPacket)?)?;

        let attachements = if let PacketId::BinaryAck | PacketId::BinaryEvent = packet_id {
            let start = i + 1;

            while payload.get(i).ok_or(Error::IncompletePacket)? != &b'-' && i < payload.len() {
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

        let nsp = if payload.get(i + 1).ok_or(Error::IncompletePacket)? == &b'/' {
            let start = i + 1;
            while payload.get(i).ok_or(Error::IncompletePacket)? != &b',' && i < payload.len() {
                i += 1;
            }
            payload
                .iter()
                .skip(start)
                .take(i - start)
                .map(|byte| *byte as char)
                .collect::<String>()
        } else {
            String::from("/")
        };

        let next = payload.get(i + 1).unwrap_or(&b'_');
        let id = if (*next as char).is_digit(10) && i < payload.len() {
            let start = i + 1;
            i += 1;
            while (*payload.get(i).ok_or(Error::IncompletePacket)? as char).is_digit(10)
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

        let mut binary_data = None;
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
                    binary_data = Some(
                        payload
                            .iter()
                            .skip(end)
                            .take(payload.len() - end)
                            .copied()
                            .collect(),
                    );

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
            nsp,
            data,
            binary_data,
            id,
            attachements,
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
        let packet = Packet::decode_bytes("0{\"token\":\"123\"}".as_bytes().to_vec());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                String::from("/"),
                Some(String::from("{\"token\":\"123\"}")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes("0/admin,{\"token\":\"123\"}".as_bytes().to_vec());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                String::from("/admin"),
                Some(String::from("{\"token\":\"123\"}")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes("1/admin,".as_bytes().to_vec());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Disconnect,
                String::from("/admin"),
                None,
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes("2[\"hello\",1]".as_bytes().to_vec());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                String::from("/"),
                Some(String::from("[\"hello\",1]")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet =
            Packet::decode_bytes("2/admin,456[\"project:delete\",123]".as_bytes().to_vec());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                String::from("/admin"),
                Some(String::from("[\"project:delete\",123]")),
                None,
                Some(456),
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes("3/admin,456[]".as_bytes().to_vec());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Ack,
                String::from("/admin"),
                Some(String::from("[]")),
                None,
                Some(456),
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes(
            "4/admin,{\"message\":\"Not authorized\"}"
                .as_bytes()
                .to_vec(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::ConnectError,
                String::from("/admin"),
                Some(String::from("{\"message\":\"Not authorized\"}")),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes(
            "51-[\"hello\",{\"_placeholder\":true,\"num\":0}]\x01\x02\x03"
                .as_bytes()
                .to_vec(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                String::from("/"),
                Some(String::from("\"hello\"")),
                Some(vec![1, 2, 3]),
                None,
                Some(1),
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes(
            "51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]\x01\x02\x03"
                .as_bytes()
                .to_vec(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                String::from("/admin"),
                Some(String::from("\"project:delete\"")),
                Some(vec![1, 2, 3]),
                Some(456),
                Some(1),
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_bytes(
            "61-/admin,456[{\"_placeholder\":true,\"num\":0}]\x03\x02\x01"
                .as_bytes()
                .to_vec(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryAck,
                String::from("/admin"),
                None,
                Some(vec![3, 2, 1]),
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
            String::from("/"),
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
            String::from("/admin"),
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
            String::from("/admin"),
            None,
            None,
            None,
            None,
        );

        assert_eq!(packet.encode(), "1/admin,".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            String::from("/"),
            Some(String::from("[\"hello\",1]")),
            None,
            None,
            None,
        );

        assert_eq!(packet.encode(), "2[\"hello\",1]".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            String::from("/admin"),
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
            String::from("/admin"),
            Some(String::from("[]")),
            None,
            Some(456),
            None,
        );

        assert_eq!(packet.encode(), "3/admin,456[]".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::ConnectError,
            String::from("/admin"),
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
            String::from("/"),
            Some(String::from("\"hello\"")),
            Some(vec![1, 2, 3]),
            None,
            Some(1),
        );

        let mut string_part = "51-[\"hello\",{\"_placeholder\":true,\"num\":0}]"
            .to_string()
            .into_bytes();
        string_part.extend(vec![1, 2, 3]);
        assert_eq!(packet.encode(), string_part);

        let packet = Packet::new(
            PacketId::BinaryEvent,
            String::from("/admin"),
            Some(String::from("\"project:delete\"")),
            Some(vec![1, 2, 3]),
            Some(456),
            Some(1),
        );

        let mut string_part = "51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]"
            .to_string()
            .into_bytes();
        string_part.extend(vec![1, 2, 3]);
        assert_eq!(packet.encode(), string_part);

        let packet = Packet::new(
            PacketId::BinaryAck,
            String::from("/admin"),
            None,
            Some(vec![3, 2, 1]),
            Some(456),
            Some(1),
        );

        let mut string_part = "61-/admin,456[{\"_placeholder\":true,\"num\":0}]"
            .to_string()
            .into_bytes();
        string_part.extend(vec![3, 2, 1]);
        assert_eq!(packet.encode(), string_part);
    }
}
