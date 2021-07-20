use crate::error::Result;

pub trait Client {
    fn open<T: Into<String> + Clone>(&mut self, address: T) -> Result<()>;
    fn poll_cycle(&self) -> Result<()>;
}
