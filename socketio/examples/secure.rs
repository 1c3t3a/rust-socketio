use native_tls::Certificate;
use native_tls::TlsConnector;
use rust_socketio::ClientBuilder;
use std::fs::File;
use std::io::Read;

fn main() {
    // In case a trusted CA is needed that isn't in the trust chain.
    let cert_path = "ca.crt";
    let mut cert_file = File::open(cert_path).expect("Failed to open cert");
    let mut buf = vec![];
    cert_file
        .read_to_end(&mut buf)
        .expect("Failed to read cert");
    let cert: Certificate = Certificate::from_pem(&buf[..]).unwrap();

    let tls_connector = TlsConnector::builder()
        .add_root_certificate(cert)
        .build()
        .expect("Failed to build TLS Connector");

    let socket = ClientBuilder::new("https://localhost:4200")
        .tls_config(tls_connector)
        // Not strictly required for HTTPS
        .opening_header("HOST", "localhost")
        .on("error", |err, _, _| eprintln!("Error: {:#?}", err))
        .connect()
        .expect("Connection failed");

    // use the socket

    socket.disconnect().expect("Disconnect failed")
}
