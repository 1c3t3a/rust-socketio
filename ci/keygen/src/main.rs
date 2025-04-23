use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, KeyUsagePurpose,
};

// Copied from https://github.com/rustls/tokio-rustls/commit/6f7373dad47df0b071c8f0f1a03376bed08331e7/tests/certs/main.rs
fn main() {
    let domain = std::env::var("DOMAIN").expect("DOMAIN not set");
    let ip = std::env::var("IP").expect("IP not set");
    let ca_name = std::env::var("CA_NAME").unwrap_or_else(|_| "rust-socketio-dev".to_string());

    let root_key = KeyPair::generate().unwrap();
    let root_ca = issuer_params(&ca_name).self_signed(&root_key).unwrap();

    std::fs::write("cert/ca.crt", root_ca.pem().as_bytes()).unwrap();

    let intermediate_key = KeyPair::generate().unwrap();
    let intermediate_ca = issuer_params("Socketio Root - Rung 2")
        .signed_by(&intermediate_key, &root_ca, &root_key)
        .unwrap();

    let end_entity_key = KeyPair::generate().unwrap();
    let mut end_entity_params =
        CertificateParams::new(vec![domain, ip, "localhost".to_string()]).unwrap();
    end_entity_params.is_ca = IsCa::ExplicitNoCa;
    end_entity_params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ServerAuth,
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    let end_entity = end_entity_params
        .signed_by(&end_entity_key, &intermediate_ca, &intermediate_key)
        .unwrap();

    std::fs::write(
        "cert/server.crt",
        end_entity
            .pem()
            .as_bytes()
            .iter()
            .chain(intermediate_ca.pem().as_bytes())
            .copied()
            .collect::<Vec<_>>(),
    )
    .unwrap();

    std::fs::write("cert/server.key", end_entity_key.serialize_pem()).unwrap();
}

fn issuer_params(common_name: &str) -> CertificateParams {
    let mut issuer_name = DistinguishedName::new();
    issuer_name.push(DnType::CommonName, common_name);
    let mut issuer_params = CertificateParams::default();
    issuer_params.distinguished_name = issuer_name;
    issuer_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    issuer_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    issuer_params
}
