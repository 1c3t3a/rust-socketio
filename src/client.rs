use crate::error::Result;

pub trait Client {
    fn connect<T: Into<String> + Clone>(&mut self, address: T) -> Result<()>;
    fn disconnect(&mut self) -> Result<()>;
}
