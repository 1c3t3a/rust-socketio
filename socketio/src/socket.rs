use crate::error::{Error, Result};
use crate::packet::{Packet, PacketId};
use bytes::Bytes;
use rust_engineio::{Client as EngineClient, Packet as EnginePacket, PacketId as EnginePacketId};
use serde::Serialize;
use serde_json::{json, Value};
use std::convert::TryFrom;
use std::sync::{atomic::AtomicBool, Arc};
use std::{fmt::Debug, sync::atomic::Ordering};

use super::{event::Event, payload::Payload, payload::RawPayload};

/// Handles communication in the `socket.io` protocol.
#[derive(Clone, Debug)]
pub(crate) struct Socket {
    //TODO: 0.4.0 refactor this
    engine_client: Arc<EngineClient>,
    connected: Arc<AtomicBool>,
}

#[derive(Serialize)]
struct BinaryPlaceHolder {
    _placeholder: bool,
    num: usize,
}

impl BinaryPlaceHolder {
    fn new(num: usize) -> Self {
        Self {
            _placeholder: true,
            num,
        }
    }
}

impl Socket {
    /// Creates an instance of `Socket`.

    pub(super) fn new(engine_client: EngineClient) -> Result<Self> {
        Ok(Socket {
            engine_client: Arc::new(engine_client),
            connected: Arc::new(AtomicBool::default()),
        })
    }

    /// Connects to the server. This includes a connection of the underlying
    /// engine.io client and afterwards an opening socket.io request.
    pub fn connect(&self) -> Result<()> {
        self.engine_client.connect()?;

        // store the connected value as true, if the connection process fails
        // later, the value will be updated
        self.connected.store(true, Ordering::Release);

        Ok(())
    }

    /// Disconnects from the server by sending a socket.io `Disconnect` packet. This results
    /// in the underlying engine.io transport to get closed as well.
    pub fn disconnect(&self) -> Result<()> {
        if self.is_engineio_connected()? {
            self.engine_client.disconnect()?;
        }
        if self.connected.load(Ordering::Acquire) {
            self.connected.store(false, Ordering::Release);
        }
        Ok(())
    }

    /// Sends a `socket.io` packet to the server using the `engine.io` client.
    pub fn send(&self, packet: Packet) -> Result<()> {
        if !self.is_engineio_connected()? || !self.connected.load(Ordering::Acquire) {
            return Err(Error::IllegalActionBeforeOpen());
        }

        // the packet, encoded as an engine.io message packet
        let engine_packet = EnginePacket::new(EnginePacketId::Message, Bytes::from(&packet));
        self.engine_client.emit(engine_packet)?;

        if let Some(attachments) = packet.attachments {
            for attachment in attachments {
                let engine_packet = EnginePacket::new(EnginePacketId::MessageBinary, attachment);
                self.engine_client.emit(engine_packet)?;
            }
        }

        Ok(())
    }

    /// Emits to certain event with given data.
    pub fn emit(&self, nsp: &str, event: Event, data: Payload) -> Result<()> {
        let socket_packet = Self::build_packet_for_payload(data, Some(event), nsp, None, false)?;
        self.send(socket_packet)
    }

    /// Returns a packet for a payload, could be used for bot binary and non binary
    /// events and acks. Convenance method.
    #[inline]
    pub(crate) fn build_packet_for_payload(
        payload: Payload,
        event: Option<Event>,
        nsp: &str,
        id: Option<i32>,
        is_ack: bool,
    ) -> Result<Packet> {
        let (data, attachments) = Self::encode_data(event, payload);

        let packet_type = match attachments.is_empty() {
            true if is_ack => PacketId::Ack,
            true => PacketId::Event,
            false if is_ack => PacketId::BinaryAck,
            false => PacketId::BinaryEvent,
        };

        Ok(Packet::new(
            packet_type,
            nsp.to_owned(),
            Some(data),
            id,
            attachments.len() as u8,
            Some(attachments),
        ))
    }

    fn encode_data(event: Option<Event>, payload: Payload) -> (Value, Vec<Bytes>) {
        let mut attachments = vec![];
        let mut data: Vec<Value> = vec![];

        if let Some(event) = event {
            data.push(json!(String::from(event)));
        }

        Self::process_payload(&mut data, payload, &mut attachments);

        let data = Value::Array(data);

        (data, attachments)
    }

    fn process_payload(data: &mut Vec<Value>, payload: Payload, attachments: &mut Vec<Bytes>) {
        match payload {
            Payload::Binary(bin_data) => Self::process_binary(data, bin_data, attachments),
            Payload::Json(value) => data.push(value),
            Payload::Multi(payloads) => {
                for payload in payloads {
                    match payload {
                        RawPayload::Binary(bin) => Self::process_binary(data, bin, attachments),
                        RawPayload::Json(value) => data.push(value),
                    }
                }
            }
        };
    }

    fn process_binary(data: &mut Vec<Value>, bin_data: Bytes, attachments: &mut Vec<Bytes>) {
        let place_holder = BinaryPlaceHolder::new(attachments.len());
        data.push(json!(&place_holder));
        attachments.push(bin_data);
    }

    pub(crate) fn poll(&self) -> Result<Option<Packet>> {
        loop {
            match self.engine_client.poll() {
                Ok(Some(packet)) => {
                    if packet.packet_id == EnginePacketId::Message
                        || packet.packet_id == EnginePacketId::MessageBinary
                    {
                        let packet = self.handle_engineio_packet(packet)?;
                        self.handle_socketio_packet(&packet);
                        return Ok(Some(packet));
                    } else {
                        continue;
                    }
                }
                Ok(None) => {
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            }
        }
    }

    /// Handles the connection/disconnection.
    #[inline]
    fn handle_socketio_packet(&self, socket_packet: &Packet) {
        match socket_packet.packet_type {
            PacketId::Connect => {
                self.connected.store(true, Ordering::Release);
            }
            PacketId::ConnectError => {
                self.connected.store(false, Ordering::Release);
            }
            PacketId::Disconnect => {
                self.connected.store(false, Ordering::Release);
            }
            _ => (),
        }
    }

    /// Handles new incoming engineio packets
    fn handle_engineio_packet(&self, packet: EnginePacket) -> Result<Packet> {
        let mut socket_packet = Packet::try_from(&packet.data)?;

        // Only handle attachments if there are any
        if socket_packet.attachment_count > 0 {
            let mut attachments_left = socket_packet.attachment_count;
            let mut attachments = Vec::new();
            while attachments_left > 0 {
                let next = self.engine_client.poll();
                match next {
                    Err(err) => return Err(err.into()),
                    Ok(Some(packet)) => match packet.packet_id {
                        EnginePacketId::MessageBinary | EnginePacketId::Message => {
                            attachments.push(packet.data);
                            attachments_left -= 1;
                        }
                        _ => {
                            return Err(Error::InvalidAttachmentPacketType(
                                packet.packet_id.into(),
                            ));
                        }
                    },
                    Ok(None) => {
                        // Engineio closed before attachments completed.
                        return Err(Error::IncompletePacket());
                    }
                }
            }
            socket_packet.attachments = Some(attachments);
        }

        Ok(socket_packet)
    }

    fn is_engineio_connected(&self) -> Result<bool> {
        Ok(self.engine_client.is_connected()?)
    }
}

#[cfg(test)]
mod test {
    use serde_json::json;

    use super::*;

    #[test]
    fn test_build_multiple_payloads_packet() {
        let payload = Payload::Multi(vec![Bytes::from_static(&[1, 2, 3]).into()]);
        let packet =
            Socket::build_packet_for_payload(payload, Some("hello".into()), "/", None, false);

        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "51-[\"hello\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let payload = Payload::Multi(vec![Bytes::from_static(&[1, 2, 3]).into()]);
        let packet = Socket::build_packet_for_payload(
            payload,
            Some("project:delete".into()),
            "/admin",
            Some(456),
            false,
        );

        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "51-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let payload = Payload::Multi(vec![Bytes::from_static(&[3, 2, 1]).into()]);
        let packet = Socket::build_packet_for_payload(payload, None, "/admin", Some(456), true);

        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "61-/admin,456[{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );

        let payload = Payload::Multi(vec![]);
        let packet =
            Socket::build_packet_for_payload(payload, Some("hello".into()), "/", None, false);
        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "2[\"hello\"]".to_string().into_bytes()
        );

        let payloads = Payload::Multi(vec![Bytes::from_static(&[1, 2, 3]).into(), json!(1).into()]);
        let packet =
            Socket::build_packet_for_payload(payloads, Some("hello".into()), "/", None, false);

        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "51-[\"hello\",{\"_placeholder\":true,\"num\":0},1]"
                .to_string()
                .into_bytes()
        );

        let payloads = Payload::Multi(vec![
            Bytes::from_static(&[1, 2, 3]).into(),
            Bytes::from_static(&[1, 2, 3]).into(),
        ]);
        let packet = Socket::build_packet_for_payload(
            payloads,
            Some("project:delete".into()),
            "/admin",
            Some(456),
            false,
        );

        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "52-/admin,456[\"project:delete\",{\"_placeholder\":true,\"num\":0},{\"_placeholder\":true,\"num\":1}]"
                .to_string()
                .into_bytes()
        );

        let payloads = Payload::Multi(vec![
            json!(3).into(),
            json!("4").into(),
            Bytes::from_static(&[3, 2, 1]).into(),
        ]);
        let packet = Socket::build_packet_for_payload(payloads, None, "/admin", Some(456), true);

        assert_eq!(
            Bytes::from(&packet.unwrap()),
            "61-/admin,456[3,\"4\",{\"_placeholder\":true,\"num\":0}]"
                .to_string()
                .into_bytes()
        );
    }
}
