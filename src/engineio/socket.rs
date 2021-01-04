use super::packet::{Error, Packet};
use crate::engineio::transport::TransportClient;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

/// An engineio socket that manages the connection with the server
/// and allows to register the common callbacks.
#[derive(Clone)]
pub struct EngineSocket {
    transport_client: Arc<RwLock<TransportClient>>,
    serving: Arc<AtomicBool>,
}

impl EngineSocket {
    /// Creates an instance.
    pub fn new(engine_io_mode: bool) -> Self {
        EngineSocket {
            transport_client: Arc::new(RwLock::new(TransportClient::new(engine_io_mode))),
            serving: Arc::new(AtomicBool::default()),
        }
    }

    /// Binds the socket to a certain address. Attention! This doesn't allow to configure
    /// callbacks afterwards.
    pub async fn bind(&self, address: String) -> Result<(), Error> {
        self.transport_client.write().unwrap().open(address).await?;

        let cl = Arc::clone(&self.transport_client);
        tokio::spawn(async move {
            let s = cl.read().unwrap().clone();
            s.poll_cycle().await.unwrap();
        });
        self.serving.swap(true, Ordering::SeqCst);

        Ok(())
    }

    /// Sends a packet to the server.
    pub async fn emit(&mut self, packet: Packet) -> Result<(), Error> {
        if !self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.read().unwrap().emit(packet).await
    }

    /// Registers the on_open callback.
    pub fn on_open<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.write().unwrap().set_on_open(function);
        Ok(())
    }

    /// Registers the on_close callback.
    pub fn on_close<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(()) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client
            .write()
            .unwrap()
            .set_on_close(function);
        Ok(())
    }

    /// Registers the on_packet callback.
    pub fn on_packet<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(Packet) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client
            .write()
            .unwrap()
            .set_on_packet(function);
        Ok(())
    }

    /// Registers the on_data callback.
    pub fn on_data<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(Vec<u8>) + 'static + Sync + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.write().unwrap().set_on_data(function);
        Ok(())
    }

    /// Registers the on_error callback.
    pub fn on_error<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(String) + 'static + Sync + Send + Send,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client
            .write()
            .unwrap()
            .set_on_error(function);
        Ok(())
    }
}

#[cfg(test)]
mod test {

    use std::time::Duration;

    use crate::engineio::packet::PacketId;

    use super::*;

    #[actix_rt::test]
    async fn test_basic_connection() {
        let mut socket = EngineSocket::new(true);

        socket
            .on_open(|_| {
                println!("Open event!");
            })
            .unwrap();

        socket
            .on_close(|_| {
                println!("Close event!");
            })
            .unwrap();

        socket
            .on_packet(|packet| {
                println!("Received packet: {:?}", packet);
            })
            .unwrap();

        socket
            .on_data(|data| {
                println!("Received packet: {:?}", std::str::from_utf8(&data));
            })
            .unwrap();

        socket
            .bind(String::from("http://localhost:4200"))
            .await
            .unwrap();

        socket
            .emit(Packet::new(
                PacketId::Message,
                "Hello World".to_string().into_bytes(),
            ))
            .await
            .unwrap();

        socket
            .emit(Packet::new(
                PacketId::Message,
                "Hello World2".to_string().into_bytes(),
            ))
            .await
            .unwrap();

        socket
            .emit(Packet::new(PacketId::Pong, Vec::new()))
            .await
            .unwrap();

        socket
            .emit(Packet::new(PacketId::Ping, Vec::new()))
            .await
            .unwrap();

        tokio::time::delay_for(Duration::from_secs(26)).await;

        socket
            .emit(Packet::new(
                PacketId::Message,
                "Hello World3".to_string().into_bytes(),
            ))
            .await
            .unwrap();
    }
}
