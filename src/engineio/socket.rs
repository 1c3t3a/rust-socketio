use crate::engineio::transport::TransportClient;
use std::sync::Arc;
use tokio::runtime::Handle;
use super::{packet::{Error, Packet}, transport};
use std::sync::Mutex;

#[derive(Clone)]
struct Socket {
    transport_client: Arc<TransportClient>,
}

impl Socket {
    async fn new(address: String) -> Self {
        let mut transport_client = Arc::new(TransportClient::new());

        Arc::get_mut(&mut transport_client).unwrap().open(address).await.unwrap();
        
        let handle = Handle::current();

        let mut cl = Arc::clone(&transport_client);
        tokio::spawn(async move {
            println!("Polling");
            Arc::get_mut(&mut cl).unwrap().poll_cycle().await.unwrap();
        });

        Socket { transport_client }
    }

    pub async fn emit(&mut self, packet: Packet) -> Result<(), Error> {
        Arc::get_mut(&mut self.transport_client).unwrap().emit(packet).await
    }

    pub fn on_open<F>(&mut self, function: F)
    where
        F: Fn(()) + 'static,
    {
        Arc::get_mut(&mut self.transport_client).unwrap().set_on_open(function);
    }

    pub fn on_close<F>(&mut self, function: F)
    where
        F: Fn(()) + 'static,
    {
        Arc::get_mut(&mut self.transport_client).unwrap().set_on_close(function);
    }

    pub fn on_packet<F>(&mut self, function: F)
    where
        F: Fn(Packet) + 'static,
    {
        Arc::get_mut(&mut self.transport_client).unwrap().set_on_packet(function);
    }

    pub fn on_data<F>(&mut self, function: F)
    where
        F: Fn(Vec<u8>) + 'static,
    {
        Arc::get_mut(&mut self.transport_client).unwrap().set_on_data(function);
    }

    pub fn on_error<F>(&mut self, function: F)
    where
        F: Fn(String) + 'static,
    {
        Arc::get_mut(&mut self.transport_client).unwrap().set_on_error(function);
    }
}

#[cfg(test)]
mod test {
    use crate::engineio::packet::PacketId;

    use super::*;

    #[actix_rt::test]
    async fn test_basic_connection() {
        let mut socket = Socket::new(String::from("http://localhost:4200")).await;

        println!("Started!");
        /**socket.on_open(|_| {
            println!("Connected!");
        });

        socket.on_packet(|packet| {
            println!("Received packet: {:?}", packet);
        });

        socket.on_data(|data| {
            println!("Received data: {}", std::str::from_utf8(&data).unwrap());
        });*/
        socket.emit(Packet::new(
            PacketId::Message,
            "Hello World".to_string().into_bytes(),
        )).await.unwrap();
    }
}
