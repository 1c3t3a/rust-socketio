#![allow(unused)]
use std::{
    ops::Deref,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};

use super::{Client, ClientBuilder};
use crate::{error::Result, packet::Packet, Error};
pub use crate::{event::Event, payload::Payload};
use backoff::ExponentialBackoff;
use backoff::{backoff::Backoff, ExponentialBackoffBuilder};

#[derive(Clone)]
pub struct ReconnectClient {
    builder: ClientBuilder,
    client: Arc<RwLock<Client>>,
    backoff: ExponentialBackoff,
}

impl ReconnectClient {
    pub(crate) fn new(builder: ClientBuilder) -> Result<Self> {
        let builder_clone = builder.clone();
        let client = builder_clone.connect_manual()?;
        let backoff = ExponentialBackoffBuilder::new()
            .with_initial_interval(Duration::from_millis(builder.reconnect_delay_min))
            .with_max_interval(Duration::from_millis(builder.reconnect_delay_max))
            .build();

        let s = Self {
            builder,
            client: Arc::new(RwLock::new(client)),
            backoff,
        };
        s.poll_callback();

        Ok(s)
    }

    pub fn emit<E, D>(&self, event: E, data: D) -> Result<()>
    where
        E: Into<Event>,
        D: Into<Payload>,
    {
        let client = self.client.read()?;
        // TODO(#230): like js client, buffer emit, resend after reconnect
        client.emit(event, data)
    }

    pub fn emit_with_ack<F, E, D>(
        &self,
        event: E,
        data: D,
        timeout: Duration,
        callback: F,
    ) -> Result<()>
    where
        F: for<'a> FnMut(Payload, Client) + 'static + Sync + Send,
        E: Into<Event>,
        D: Into<Payload>,
    {
        let client = self.client.read()?;
        // TODO(#230): like js client, buffer emit, resend after reconnect
        client.emit_with_ack(event, data, timeout, callback)
    }

    pub fn disconnect(&self) -> Result<()> {
        let client = self.client.read()?;
        client.disconnect()
    }

    fn reconnect(&mut self) {
        let mut reconnect_attempts = 0;
        if self.builder.reconnect {
            loop {
                if let Some(max_reconnect_attempts) = self.builder.max_reconnect_attempts {
                    if reconnect_attempts > max_reconnect_attempts {
                        break;
                    }
                }
                reconnect_attempts += 1;

                if let Some(backoff) = self.backoff.next_backoff() {
                    std::thread::sleep(backoff);
                }

                if self.do_reconnect().is_ok() {
                    break;
                }
            }
        }
    }

    fn do_reconnect(&self) -> Result<()> {
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
        let mut self_clone = self.clone();
        // Use thread to consume items in iterator in order to call callbacks
        std::thread::spawn(move || {
            // tries to restart a poll cycle whenever a 'normal' error occurs,
            // it just panics on network errors, in case the poll cycle returned
            // `Result::Ok`, the server receives a close frame so it's safe to
            // terminate
            for packet in self_clone.iter() {
                if let e @ Err(Error::IncompleteResponseFromEngineIo(_)) = packet {
                    //TODO: 0.3.X handle errors
                    //TODO: logging error
                    let _ = self_clone.disconnect();
                    self_clone.reconnect();
                }
            }
        });
    }
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

#[cfg(test)]
mod test {
    use std::sync::mpsc;
    use std::thread::sleep;

    use super::*;
    use crate::error::Result;
    use crate::{client::TransportType, payload::Payload, ClientBuilder};
    use bytes::Bytes;
    use native_tls::TlsConnector;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[test]
    fn socket_io_reconnect_integration() -> Result<()> {
        let url = crate::test::socket_io_restart_server();
        let connect_count = Arc::new(Mutex::new(0));
        let close_count = Arc::new(Mutex::new(0));
        let message_count = Arc::new(Mutex::new(0));
        let connect_count_clone = connect_count.clone();
        let close_count_clone = close_count.clone();
        let message_count_clone = message_count.clone();

        let socket = ClientBuilder::new(url)
            .reconnect(true)
            .max_reconnect_attempts(100)
            .reconnect_delay(100, 100)
            .on(Event::Connect, move |_, socket| {
                let mut connect_cnt = connect_count_clone.lock().unwrap();
                *connect_cnt += 1;

                let r = socket.emit_with_ack(
                    "message",
                    json!(""),
                    Duration::from_millis(100),
                    |_, _| {},
                );
                assert!(r.is_ok(), "should emit message success");
            })
            .on(Event::Close, move |_, _| {
                let mut close_cnt = close_count_clone.lock().unwrap();
                *close_cnt += 1;
            })
            .on("message", move |_, socket| {
                // test the iterator implementation and make sure there is a constant
                // stream of packets, even when reconnecting
                let mut message_cnt = message_count_clone.lock().unwrap();
                *message_cnt += 1;
            })
            .connect();

        assert!(socket.is_ok(), "should connect success");
        let socket = socket.unwrap();

        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            {
                let message_cnt = message_count.lock().unwrap();
                if *message_cnt == 1 {
                    break;
                }
            }
        }

        {
            let connect_cnt = connect_count.lock().unwrap();
            let message_cnt = message_count.lock().unwrap();
            let close_cnt = close_count.lock().unwrap();
            assert_eq!(*connect_cnt, 1, "should connect once");
            assert_eq!(*message_cnt, 1, "should receive message ");
            assert_eq!(*close_cnt, 0, "should not close");
        }

        let r = socket.emit("restart_server", json!(""));
        assert!(r.is_ok(), "should emit restart success");

        let now = Instant::now();
        for _ in 0..10 {
            std::thread::sleep(std::time::Duration::from_millis(400));
            {
                let message_cnt = message_count.lock().unwrap();
                let connect_cnt = connect_count.lock().unwrap();
                if *message_cnt == 2 && *connect_cnt == 2 {
                    break;
                }
            }
        }

        {
            let connect_cnt = connect_count.lock().unwrap();
            let message_cnt = message_count.lock().unwrap();
            let close_cnt = close_count.lock().unwrap();
            assert_eq!(*connect_cnt, 2, "should connect twice {:?}", now.elapsed());
            assert_eq!(*message_cnt, 2, "should receive two messages");
            assert_eq!(*close_cnt, 1, "should close once");
        }

        socket.disconnect()?;
        Ok(())
    }
}
