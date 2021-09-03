use crate::error::Result;
use crate::Packet;

pub trait Socket: Send + Sync + Clone {
    fn close(&self) -> Result<()>;
    fn connect(&self) -> Result<()>;
    fn emit(&self, packet: Packet) -> Result<()>;
    fn is_connected(&self) -> Result<bool>;
}
