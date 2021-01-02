use crate::engineio::packet::Error;
use either::*;

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

struct Packet {
    packet_type: PacketId,
    nsp: String,
    data: Option<Vec<Either<String, Vec<u8>>>>,
    id: Option<i32>,
    attachements: Option<u8>,
}

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
    fn new(
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
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    /// This test suites comes from the explanation section here: https://github.com/socketio/socket.io-protocol
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
