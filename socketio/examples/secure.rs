use rust_socketio::ClientBuilder;

fn main() {
    // In case a trusted CA is needed that isn't in the trust chain.
    let cert_path = "ca.crt";
    let buf = std::fs::read(cert_path).expect("Failed to open cert");

    // Create a socket.io client
    let socket = ClientBuilder::new("https://localhost:4200")
        .tls_config(tls_connector(&buf))
        // Not strictly required for HTTPS
        .opening_header("HOST", "localhost")
        .on("error", |err, _| eprintln!("Error: {:#?}", err))
        .connect()
        .expect("Connection failed");

    // use the socket
    socket.disconnect().expect("Disconnect failed")
}

fn tls_connector(buf: &[u8]) -> rust_socketio::TlsConfig {
    #[cfg(all(feature = "_native-tls", not(feature = "_rustls-tls")))]
    {
        let cert = native_tls::Certificate::from_pem(buf).unwrap();
        native_tls::TlsConnector::builder()
            // ONLY USE FOR TESTING!
            .danger_accept_invalid_hostnames(true)
            .add_root_certificate(cert)
            .build()
            .unwrap()
    }
    #[cfg(feature = "_rustls-tls")]
    {
        let mut root_store = rustls::RootCertStore::empty();
        for cert in rustls_pemfile::certs(&mut &buf[..]) {
            root_store
                .add(cert.expect("Invalid PEM cert"))
                .expect("Failed to add cert to store");
        }
        let mut config = rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        config.enable_sni = false;
        config
    }
}
