#![allow(unused)]
use std::sync::{Arc, RwLock};

use super::{Client, ClientBuilder};
use crate::{error::Result, packet::Packet, Error};

#[derive(Clone)]
pub(crate) struct ReconnectClient {
    builder: ClientBuilder,
    client: Arc<RwLock<super::Client>>,
}

impl ReconnectClient {
    pub(crate) fn new(builder: ClientBuilder) -> Result<Self> {
        let builder_clone = builder.clone();
        let client = builder.connect_manual()?;
        Ok(Self {
            builder: builder_clone,
            client: Arc::new(RwLock::new(client)),
        })
    }

    fn replace_client(&self) -> Result<()> {
        let new_client = self.builder.clone().connect_manual()?;
        let mut client = self.client.write()?;
        *client = new_client;
        Ok(())
    }

    pub(crate) fn iter(&self) -> Iter {
        Iter {
            socket: self.client.clone(),
        }
    }

    fn poll_callback(&self) {
        let self_clone = self.clone();
        // Use thread to consume items in iterator in order to call callbacks
        std::thread::spawn(move || {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            for packet in self_clone.iter() {
                if let e @ Err(Error::IncompleteResponseFromEngineIo(_)) = packet {
                    //TODO: 0.3.X handle errors

                    // TODO: implement proper backoff
                    let _ = self_clone.replace_client();
                }
            }
        });
    }

    // TODO: implement client wrapper methods here. possibly factor out into a trait
}

pub(crate) struct Iter {
    socket: Arc<RwLock<Client>>,
}

impl Iterator for Iter {
    type Item = Result<Packet>;

    fn next(&mut self) -> Option<Self::Item> {
        let socket = self.socket.read();
        match socket {
            Ok(socket) => match socket.poll() {
                Err(err) => Some(Err(err)),
                Ok(Some(packet)) => Some(Ok(packet)),
                Ok(None) => None,
            },
            Err(_) => {
                // Lock is poisoned, our iterator is useless.
                None
            }
        }
    }
}
