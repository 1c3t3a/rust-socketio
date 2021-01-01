use super::packet::{Error, Packet};
use crate::engineio::transport::TransportClient;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

#[derive(Clone)]
struct Socket {
    transport_client: Arc<RwLock<TransportClient>>,
    serving: Arc<AtomicBool>,
}

impl Socket {
    pub fn new() -> Self {
        Socket {
            transport_client: Arc::new(RwLock::new(TransportClient::new())),
            serving: Arc::new(AtomicBool::default()),
        }
    }

    pub async fn bind(&mut self, address: String) {
        self.transport_client
            .write()
            .unwrap()
            .open(address)
            .await
            .expect("Error while opening connection");

        let cl = Arc::clone(&self.transport_client);
        tokio::spawn(async move {
            let s = cl.read().unwrap().clone();
            s.poll_cycle().await.unwrap();
        });
        self.serving.swap(true, Ordering::SeqCst);
    }

    pub async fn emit(&mut self, packet: Packet) -> Result<(), Error> {
        if !self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.read().unwrap().emit(packet).await
    }

    pub fn on_open<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(()) + 'static,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.write().unwrap().set_on_open(function);
        Ok(())
    }

    pub fn on_close<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(()) + 'static,
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

    pub fn on_packet<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(Packet) + 'static,
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

    pub fn on_data<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(Vec<u8>) + 'static,
    {
        if self.serving.load(Ordering::Relaxed) {
            return Err(Error::ActionBeforeOpen);
        }
        self.transport_client.write().unwrap().set_on_data(function);
        Ok(())
    }

    pub fn on_error<F>(&mut self, function: F) -> Result<(), Error>
    where
        F: Fn(String) + 'static,
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
        let mut socket = Socket::new();

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

        socket.bind(String::from("http://localhost:4200")).await;

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
