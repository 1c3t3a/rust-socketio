use crate::engineio::packet::Error;
use either::*;
use regex::Regex;

/// An enumeration of the different Paccket types in the socket.io protocol.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum PacketId {
    Connect = 0,
    Disconnect = 1,
    Event = 2,
    Ack = 3,
    ConnectError = 4,
    BinaryEvent = 5,
    BinaryAck = 6,
}

/// A packet which get's send or received during in the socket-io protocol.
#[derive(Debug, PartialEq, Eq)]
pub struct Packet {
    packet_type: PacketId,
    nsp: String,
    data: Option<Vec<Either<String, Vec<u8>>>>,
    id: Option<i32>,
    attachements: Option<u8>,
}

/// Converts an u8 byte to an PacketId.
pub fn u8_to_packet_id(b: u8) -> Result<PacketId, Error> {
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
    pub fn new(
        packet_type: PacketId,
        nsp: String,
        data: Option<Vec<Either<String, Vec<u8>>>>,
        id: Option<i32>,
        attachements: Option<u8>,
    ) -> Self {
        Packet {
            packet_type,
            nsp,
            data,
            id,
            attachements,
        }
    }

    /// Encodes the packet to an u8 byte stream.
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

        // if the namespace is different from the default one append it as well, followed by ','
        if self.nsp != "/" {
            string.push_str(self.nsp.as_ref());
            string.push(',');
        }

        // if an id is present append it...
        if let Some(id) = self.id.as_ref() {
            string.push_str(&id.to_string());
        }

        let mut buffer = Vec::new();
        // ... as well as the stringified json data or the bytes
        if let Some(data) = self.data.as_ref() {
            let mut binary_packets = Vec::new();

            let contains_binary = data.iter().find(|payload| payload.is_right()).is_some();

            if contains_binary {
                string.push_str("[");
            }

            for payload in data {
                match payload {
                    Left(str) => {
                        string.push_str(str);

                        if contains_binary {
                            string.push(',');
                        }
                    }
                    Right(bin_data) => {
                        binary_packets.push(bin_data.to_owned());
                    }
                }
            }

            let number_of_bins: i8 =
                (data.iter().filter(|payload| payload.is_right()).count() as i8) - 1;

            if number_of_bins >= 0 {
                string.push_str(&format!(
                    "{{\"_placeholder\":true,\"num\":{}}}",
                    number_of_bins
                ));
            }
            if contains_binary {
                string.push_str("]");
            }

            buffer.extend(string.into_bytes());
            buffer.extend(binary_packets.into_iter().flatten().collect::<Vec<u8>>());
        } else {
            buffer.extend(string.into_bytes());
        }

        return buffer;
    }

    /// Decodes a packet given an utf-8 string.
    fn decode_string(string: String) -> Result<Self, Error> {
        let mut i = 0;
        let packet_id = u8_to_packet_id(string.as_bytes()[i])?;

        let attachements = if let PacketId::BinaryAck | PacketId::BinaryEvent = packet_id {
            let start = i + 1;

            while string.chars().nth(i).unwrap() != '-' && i < string.len() {
                i += 1;
            }
            Some(
                string
                    .chars()
                    .skip(start)
                    .take(i - start)
                    .collect::<String>()
                    .parse::<u8>()
                    .unwrap(),
            )
        } else {
            None
        };

        let nsp = if string.chars().nth(i + 1).unwrap() == '/' {
            let start = i + 1;
            while string.chars().nth(i).unwrap() != ',' && i < string.len() {
                i += 1;
            }
            string
                .chars()
                .skip(start)
                .take(i - start)
                .collect::<String>()
        } else {
            String::from("/")
        };

        let next = string.chars().nth(i + 1).unwrap_or('_');
        let id = if next.to_digit(10).is_some() && i < string.len() {
            let start = i + 1;
            i += 1;
            while string.chars().nth(i).unwrap().to_digit(10).is_some() && i < string.len() {
                i += 1;
            }

            Some(
                string
                    .chars()
                    .skip(start)
                    .take(i - start)
                    .collect::<String>()
                    .parse::<i32>()
                    .unwrap(),
            )
        } else {
            None
        };

        let data = if string.chars().nth(i + 1).is_some() {
            let start = if id.is_some() { i } else { i + 1 };

            let mut json_data = serde_json::Value::Null;

            let mut end = string.len();
            while let Err(_) = serde_json::from_str::<serde_json::Value>(
                &string
                    .chars()
                    .skip(start)
                    .take(end - start)
                    .collect::<String>(),
            ) {
                end -= 1;
            }

            if end != start {
                json_data = serde_json::from_str(
                    &string
                        .chars()
                        .skip(start)
                        .take(end - start)
                        .collect::<String>(),
                )
                .unwrap();
            }

            match packet_id {
                PacketId::BinaryAck | PacketId::BinaryEvent => {
                    let buffer = string
                        .chars()
                        .skip(end)
                        .take(string.len() - end)
                        .collect::<String>()
                        .as_bytes()
                        .to_vec();

                    let re_open = Regex::new(r"^\[").unwrap();
                    let re_close = Regex::new(r",]$|]$").unwrap();
                    let mut str = json_data
                        .to_string()
                        .replace("{\"_placeholder\":true,\"num\":0}", "");
                    str = re_open.replace(&str, "").to_string();
                    str = re_close.replace(&str, "").to_string();

                    if str.len() == 0 {
                        Some(vec![Right(buffer)])
                    } else {
                        Some(vec![Left(str), Right(buffer)])
                    }
                }
                _ => Some(vec![Left(json_data.to_string())]),
            }
        } else {
            None
        };

        Ok(Packet::new(packet_id, nsp, data, id, attachements))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    /// This test suites is taken from the explanation section here: https://github.com/socketio/socket.io-protocol
    fn test_decode() {
        let packet = Packet::decode_string("0{\"token\":\"123\"}".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                String::from("/"),
                Some(vec![Left(String::from("{\"token\":\"123\"}"))]),
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string("0/admin,{\"token\":\"123\"}".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Connect,
                String::from("/admin"),
                Some(vec![Left(String::from("{\"token\":\"123\"}"))]),
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string("1/admin,".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Disconnect,
                String::from("/admin"),
                None,
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string("2[\"hello\",1]".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                String::from("/"),
                Some(vec![Left(String::from("[\"hello\",1]"))]),
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string("2/admin,456[\"project:delete\",123]".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Event,
                String::from("/admin"),
                Some(vec![Left(String::from("[\"project:delete\",123]"))]),
                Some(456),
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string("3/admin,456[]".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::Ack,
                String::from("/admin"),
                Some(vec![Left(String::from("[]"))]),
                Some(456),
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string("4/admin,{\"message\":\"Not authorized\"}".to_string());
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::ConnectError,
                String::from("/admin"),
                Some(vec![Left(String::from("{\"message\":\"Not authorized\"}"))]),
                None,
                None,
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string(
            "51-[\"hello\",{\"_placeholder\":true,\"num\":0}]\x01\x02\x03".to_string(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                String::from("/"),
                Some(vec![Left(String::from("\"hello\"")), Right(vec![1, 2, 3])]),
                None,
                Some(1),
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string(
            "51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]\x01\x02\x03"
                .to_string(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryEvent,
                String::from("/admin"),
                Some(vec![
                    Left(String::from("\"project:delete\"")),
                    Right(vec![1, 2, 3]),
                ]),
                Some(456),
                Some(1),
            ),
            packet.unwrap()
        );

        let packet = Packet::decode_string(
            "61-/admin,456[{\"_placeholder\":true,\"num\":0}]\x03\x02\x01".to_string(),
        );
        assert!(packet.is_ok());

        assert_eq!(
            Packet::new(
                PacketId::BinaryAck,
                String::from("/admin"),
                Some(vec![Right(vec![3, 2, 1])]),
                Some(456),
                Some(1),
            ),
            packet.unwrap()
        );
    }

    #[test]
    /// This test suites is taken from the explanation section here: https://github.com/socketio/socket.io-protocol
    fn test_encode() {
        let packet = Packet::new(
            PacketId::Connect,
            String::from("/"),
            Some(vec![Left(String::from("{\"token\":\"123\"}"))]),
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
            Some(vec![Left(String::from("{\"token\":\"123\"}"))]),
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
        );

        assert_eq!(packet.encode(), "1/admin,".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            String::from("/"),
            Some(vec![Left(String::from("[\"hello\",1]"))]),
            None,
            None,
        );

        assert_eq!(packet.encode(), "2[\"hello\",1]".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::Event,
            String::from("/admin"),
            Some(vec![Left(String::from("[\"project:delete\",123]"))]),
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
            Some(vec![Left(String::from("[]"))]),
            Some(456),
            None,
        );

        assert_eq!(packet.encode(), "3/admin,456[]".to_string().into_bytes());

        let packet = Packet::new(
            PacketId::ConnectError,
            String::from("/admin"),
            Some(vec![Left(String::from("{\"message\":\"Not authorized\"}"))]),
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
            Some(vec![Left(String::from("\"hello\"")), Right(vec![1, 2, 3])]),
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
            Some(vec![
                Left(String::from("\"project:delete\"")),
                Right(vec![1, 2, 3]),
            ]),
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
            Some(vec![Right(vec![3, 2, 1])]),
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
