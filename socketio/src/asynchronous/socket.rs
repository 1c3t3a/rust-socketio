use super::generator::StreamGenerator;
use crate::{
    error::Result,
    packet::{Packet, PacketId},
    Error, Event, Payload,
};
use async_stream::try_stream;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rust_engineio::{
    asynchronous::Client as EngineClient, Packet as EnginePacket, PacketId as EnginePacketId,
};
use std::{fmt::Debug, pin::Pin, sync::Arc};

#[derive(Clone)]
pub(crate) struct Socket {
    engine_client: Arc<EngineClient>,
    generator: StreamGenerator<Packet>,
    is_server: bool,
}

impl Socket {
    /// Creates an instance of `Socket`.
    pub(super) fn new(engine_client: EngineClient) -> Result<Self> {
        Ok(Socket {
            engine_client: Arc::new(engine_client.clone()),
            generator: StreamGenerator::new(Self::stream(engine_client)),
            is_server: false,
        })
    }

    pub(super) fn new_server(engine_client: EngineClient) -> Self {
        Socket {
            engine_client: Arc::new(engine_client.clone()),
            generator: StreamGenerator::new(Self::stream(engine_client)),
            is_server: true,
        }
    }

    /// Connects to the server. This includes a connection of the underlying
    /// engine.io client and afterwards an opening socket.io request.
    pub async fn connect(&self) -> Result<()> {
        if !self.is_server {
            self.engine_client.connect().await?;
        }

        Ok(())
    }

    /// Disconnects from the server by sending a socket.io `Disconnect` packet. This results
    /// in the underlying engine.io transport to get closed as well.
    pub async fn disconnect(&self) -> Result<()> {
        if self.is_engineio_connected() {
            self.engine_client.disconnect().await?;
        }

        Ok(())
    }

    /// Sends a `socket.io` packet to the server using the `engine.io` client.
    pub async fn send(&self, packet: Packet) -> Result<()> {
        if !self.is_engineio_connected() {
            return Err(Error::IllegalActionBeforeOpen());
        }

        // the packet, encoded as an engine.io message packet
        let engine_packet = EnginePacket::new(EnginePacketId::Message, Bytes::from(&packet));
        self.engine_client.emit(engine_packet).await?;

        if let Some(attachments) = packet.attachments {
            for attachment in attachments {
                let engine_packet = EnginePacket::new(EnginePacketId::MessageBinary, attachment);
                self.engine_client.emit(engine_packet).await?;
            }
        }

        Ok(())
    }

    pub async fn ack(&self, nsp: &str, id: usize, data: Payload) -> Result<()> {
        let socket_packet = self.build_packet_for_payload(data, None, nsp, Some(id), true)?;

        self.send(socket_packet).await
    }

    /// Emits to certain event with given data. The data needs to be JSON,
    /// otherwise this returns an `InvalidJson` error.
    pub async fn emit(&self, nsp: &str, event: Event, data: Payload) -> Result<()> {
        let socket_packet = self.build_packet_for_payload(data, Some(event), nsp, None, false)?;

        self.send(socket_packet).await
    }

    pub(crate) async fn handshake(&self, nsp: &str, data: String) -> Result<()> {
        let socket_packet =
            Packet::new(PacketId::Connect, nsp.to_owned(), Some(data), None, 0, None);
        self.send(socket_packet).await
    }

    /// Returns a packet for a payload, could be used for bot binary and non binary
    /// events and acks. Convenance method.
    #[inline]
    pub(crate) fn build_packet_for_payload<'a>(
        &'a self,
        payload: Payload,
        event: Option<Event>,
        nsp: &'a str,
        id: Option<usize>,
        is_ack: bool,
    ) -> Result<Packet> {
        match payload {
            Payload::Binary(bin_data) => Ok(Packet::new(
                if id.is_some() && is_ack {
                    PacketId::BinaryAck
                } else {
                    PacketId::BinaryEvent
                },
                nsp.to_owned(),
                event.map(|event| serde_json::Value::String(event.into()).to_string()),
                id,
                1,
                Some(vec![bin_data]),
            )),
            Payload::String(str_data) => {
                serde_json::from_str::<serde_json::Value>(&str_data)?;

                let payload = match event {
                    None => format!("[{}]", str_data),
                    Some(event) => format!("[\"{}\",{}]", String::from(event), str_data),
                };

                Ok(Packet::new(
                    if id.is_some() && is_ack {
                        PacketId::Ack
                    } else {
                        PacketId::Event
                    },
                    nsp.to_owned(),
                    Some(payload),
                    id,
                    0,
                    None,
                ))
            }
        }
    }

    fn stream(client: EngineClient) -> Pin<Box<impl Stream<Item = Result<Packet>> + Send>> {
        Box::pin(try_stream! {
            for await received_data in client.clone() {
                let packet = received_data?;

                if packet.packet_id == EnginePacketId::Message || packet.packet_id == EnginePacketId::MessageBinary {
                    let packet = Self::handle_engineio_packet(packet, client.clone()).await?;

                    yield packet;
                }
            }
        })
    }

    /// Handles new incoming engineio packets
    async fn handle_engineio_packet(
        packet: EnginePacket,
        mut client: EngineClient,
    ) -> Result<Packet> {
        let socket_packet = Packet::try_from(&packet.data);
        if let Err(err) = socket_packet {
            return Err(err);
        }
        // SAFETY: checked above to see if it was Err
        let mut socket_packet = socket_packet.unwrap();
        // Only handle attachments if there are any
        if socket_packet.attachment_count > 0 {
            let mut attachments_left = socket_packet.attachment_count;
            let mut attachments = Vec::new();
            while attachments_left > 0 {
                // TODO: This is not nice! Find a different way to peek the next element while mapping the stream
                let next = client.next().await.unwrap();
                match next {
                    Err(err) => return Err(err.into()),
                    Ok(packet) => match packet.packet_id {
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
                }
            }
            socket_packet.attachments = Some(attachments);
        }

        Ok(socket_packet)
    }

    fn is_engineio_connected(&self) -> bool {
        self.engine_client.is_connected()
    }
}

impl Stream for Socket {
    type Item = Result<Packet>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.generator.poll_next_unpin(cx)
    }
}

impl Debug for Socket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Socket")
            .field("engine_client", &self.engine_client)
            .field("is_server", &self.is_server)
            .field("connected", &self.is_engineio_connected())
            .finish()
    }
}
