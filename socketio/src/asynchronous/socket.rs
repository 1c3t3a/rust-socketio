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
use std::{
    fmt::Debug,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone)]
pub(crate) struct Socket {
    engine_client: Arc<EngineClient>,
    connected: Arc<AtomicBool>,
    generator: StreamGenerator<Packet>,
}

impl Socket {
    /// Creates an instance of `Socket`.
    pub(super) fn new(engine_client: EngineClient) -> Result<Self> {
        let connected = Arc::new(AtomicBool::default());
        Ok(Socket {
            engine_client: Arc::new(engine_client.clone()),
            connected: connected.clone(),
            generator: StreamGenerator::new(Self::stream(engine_client, connected)),
        })
    }

    /// Connects to the server. This includes a connection of the underlying
    /// engine.io client and afterwards an opening socket.io request.
    pub async fn connect(&self) -> Result<()> {
        self.engine_client.connect().await?;

        // store the connected value as true, if the connection process fails
        // later, the value will be updated
        self.connected.store(true, Ordering::Release);

        Ok(())
    }

    /// Disconnects from the server by sending a socket.io `Disconnect` packet. This results
    /// in the underlying engine.io transport to get closed as well.
    pub async fn disconnect(&self) -> Result<()> {
        if self.is_engineio_connected() {
            self.engine_client.disconnect().await?;
        }
        if self.connected.load(Ordering::Acquire) {
            self.connected.store(false, Ordering::Release);
        }
        Ok(())
    }

    /// Sends a `socket.io` packet to the server using the `engine.io` client.
    pub async fn send(&self, packet: Packet) -> Result<()> {
        if !self.is_engineio_connected() || !self.connected.load(Ordering::Acquire) {
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

    /// Emits to certain event with given data. The data needs to be JSON,
    /// otherwise this returns an `InvalidJson` error.
    pub async fn emit(&self, nsp: &str, event: Event, data: Payload) -> Result<()> {
        let socket_packet = Packet::new_from_payload(data, event, nsp, None)?;

        self.send(socket_packet).await
    }

    fn stream(
        mut client: EngineClient,
        is_connected: Arc<AtomicBool>,
    ) -> Pin<Box<impl Stream<Item = Result<Packet>> + Send>> {
        Box::pin(try_stream! {
            loop {
                let received_data = match client.next().await {
                    Some(result) => result,
                    None => break,
                };
                let packet = received_data?;

                if packet.packet_id == EnginePacketId::Message
                    || packet.packet_id == EnginePacketId::MessageBinary
                {
                    let mut socket_packet = Packet::try_from(&packet.data)?;

                    // Collect binary attachments inline from the same stream.
                    // This fixes a race condition where `client.clone()` caused
                    // the outer loop and the attachment loop to compete for packets,
                    // resulting in binary WebSocket frames being misinterpreted as
                    // Socket.IO text packets (causing EngineIO Error / InvalidUtf8).
                    if socket_packet.attachment_count > 0 {
                        let mut attachments_left = socket_packet.attachment_count;
                        let mut attachments = Vec::new();
                        while attachments_left > 0 {
                            let next = match client.next().await {
                                Some(result) => result,
                                None => Err(Error::IncompletePacket())?,
                            };
                            match next {
                                Err(err) => Err(err)?,
                                Ok(att_packet) => match att_packet.packet_id {
                                    EnginePacketId::MessageBinary | EnginePacketId::Message => {
                                        attachments.push(att_packet.data);
                                        attachments_left -= 1;
                                    }
                                    _ => {
                                        Err(Error::InvalidAttachmentPacketType(
                                            att_packet.packet_id.into(),
                                        ))?;
                                    }
                                },
                            }
                        }
                        socket_packet.attachments = Some(attachments);
                    }

                    Self::handle_socketio_packet(&socket_packet, is_connected.clone());
                    yield socket_packet;
                }
            }
        })
    }

    /// Handles the connection/disconnection.
    #[inline]
    fn handle_socketio_packet(socket_packet: &Packet, is_connected: Arc<AtomicBool>) {
        match socket_packet.packet_type {
            PacketId::Connect => {
                is_connected.store(true, Ordering::Release);
            }
            PacketId::ConnectError => {
                is_connected.store(false, Ordering::Release);
            }
            PacketId::Disconnect => {
                is_connected.store(false, Ordering::Release);
            }
            _ => (),
        }
    }

    // Note: handle_engineio_packet was removed and its logic inlined into stream()
    // to fix a race condition where client.clone() caused the outer iteration loop
    // and the binary attachment collection loop to compete for the same packets.

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
            .field("connected", &self.connected)
            .finish()
    }
}
