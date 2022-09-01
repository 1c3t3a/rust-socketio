mod polling;
mod websocket;
mod websocket_general;
mod websocket_secure;

pub use self::polling::PollingTransport;
pub use self::websocket::WebsocketTransport;
pub use self::websocket_secure::WebsocketSecureTransport;

use futures_util::stream::{SplitSink, SplitStream};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tungstenite::Message;

type AsyncWebsocketSender = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type AsyncWebsocketReceiver = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;
