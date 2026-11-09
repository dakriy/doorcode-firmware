//! A certificate authority made for one test, and a server certificate it
//! signed for `localhost` and `127.0.0.1`.

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};

pub(crate) struct Pki {
    pub(crate) ca_pem: String,
    pub(crate) server_cert_pem: String,
    pub(crate) server_key_pem: String,
}

pub(crate) fn generate() -> Pki {
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "connector test CA");
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let server_key = KeyPair::generate().unwrap();
    let server_params =
        CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()]).unwrap();
    let server_cert = server_params.signed_by(&server_key, &issuer).unwrap();
    Pki {
        ca_pem: ca_cert.pem(),
        server_cert_pem: server_cert.pem(),
        server_key_pem: server_key.serialize_pem(),
    }
}
