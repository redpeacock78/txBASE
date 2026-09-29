use super::*;
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    generate_simple_self_signed,
};
use rustls::{ClientConfig, ClientConnection, RootCertStore, pki_types::PrivateKeyDer};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    thread,
    time::Duration,
};
use tiny_http::{Header, Response};

static NEXT_TLS_TEST_ID: AtomicUsize = AtomicUsize::new(0);

struct TestCertificates {
    directory: PathBuf,
    config: CatalogTlsConfig,
    client_config: Arc<ClientConfig>,
    missing_client_config: Arc<ClientConfig>,
    untrusted_client_config: Arc<ClientConfig>,
}

impl Drop for TestCertificates {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn test_certificates() -> TestCertificates {
    let id = NEXT_TLS_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let directory =
        std::env::temp_dir().join(format!("txbase-catalog-tls-{}-{id}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();

    let server = generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let server_cert = server.cert.der().clone();
    let certificate = directory.join("server.pem");
    let private_key = directory.join("server-key.pem");
    fs::write(&certificate, server.cert.pem()).unwrap();
    fs::write(&private_key, server.key_pair.serialize_pem()).unwrap();

    let mut ca_params = CertificateParams::default();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca_key = KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let ca_path = directory.join("client-ca.pem");
    fs::write(&ca_path, ca.pem()).unwrap();

    let mut client_params = CertificateParams::default();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_key = KeyPair::generate().unwrap();
    let client_cert = client_params.signed_by(&client_key, &ca, &ca_key).unwrap();
    let mut server_roots = RootCertStore::empty();
    server_roots.add(server_cert.clone()).unwrap();
    let client_config = ClientConfig::builder()
        .with_root_certificates(server_roots.clone())
        .with_client_auth_cert(
            vec![client_cert.der().clone()],
            PrivateKeyDer::Pkcs8(client_key.serialize_der().into()),
        )
        .unwrap();
    let missing_client_config = ClientConfig::builder()
        .with_root_certificates(server_roots.clone())
        .with_no_client_auth();

    let mut untrusted_params = CertificateParams::default();
    untrusted_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let untrusted_key = KeyPair::generate().unwrap();
    let untrusted_cert = untrusted_params.self_signed(&untrusted_key).unwrap();
    let untrusted_client_config = ClientConfig::builder()
        .with_root_certificates(server_roots)
        .with_client_auth_cert(
            vec![untrusted_cert.der().clone()],
            PrivateKeyDer::Pkcs8(untrusted_key.serialize_der().into()),
        )
        .unwrap();

    TestCertificates {
        directory,
        config: CatalogTlsConfig {
            certificate,
            private_key,
            client_ca: Some(ca_path),
        },
        client_config: Arc::new(client_config),
        missing_client_config: Arc::new(missing_client_config),
        untrusted_client_config: Arc::new(untrusted_client_config),
    }
}

fn completes_tls_handshake<A: ToSocketAddrs>(address: A, config: Arc<ClientConfig>) -> bool {
    let mut socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let server_name = "localhost".try_into().unwrap();
    let mut connection = ClientConnection::new(config, server_name).unwrap();
    while connection.is_handshaking() {
        if connection.complete_io(&mut socket).is_err() {
            return false;
        }
    }
    true
}

fn request<A: ToSocketAddrs>(address: A, bytes: &[u8]) -> Vec<u8> {
    let mut socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    socket.write_all(bytes).unwrap();
    read_http_response(&mut socket)
}

fn read_http_response(stream: &mut impl Read) -> Vec<u8> {
    let mut response = Vec::new();
    let mut byte = [0_u8; 1];
    while !response.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        response.push(byte[0]);
    }
    let headers = String::from_utf8_lossy(&response);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let start = response.len();
    response.resize(start + content_length, 0);
    stream.read_exact(&mut response[start..]).unwrap();
    response
}

fn send_tls_request(address: SocketAddr, config: Arc<ClientConfig>, request: &[u8]) -> Vec<u8> {
    let socket = TcpStream::connect(address).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let server_name = "localhost".try_into().unwrap();
    let connection = ClientConnection::new(config, server_name).unwrap();
    let mut stream = rustls::StreamOwned::new(connection, socket);
    stream.write_all(request).unwrap();
    read_http_response(&mut stream)
}

#[test]
fn public_listener_requires_trusted_client_cert_and_protects_the_http_backend() {
    let certificates = test_certificates();
    let listener = CatalogListener::bind("127.0.0.1:0", Some(&certificates.config)).unwrap();
    let backend_addr = listener.server.server_addr().to_ip().unwrap();
    let backend = Arc::clone(&listener.server);
    let token = listener.token.clone().unwrap();
    let backend_thread = thread::spawn(move || {
        let bypass = backend
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(!request_has_internal_token(&bypass, Some(&token)));
        bypass
            .respond(Response::from_string("Forbidden").with_status_code(403))
            .unwrap();

        let forwarded = backend
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(request_has_internal_token(&forwarded, Some(&token)));
        assert_eq!(forwarded.url(), "/catalog?mode=full");
        assert!(
            !forwarded
                .headers()
                .iter()
                .any(|header| header.field.equiv("x-hop-secret"))
        );
        assert!(
            !forwarded
                .headers()
                .iter()
                .any(|header| header.field.equiv("connection"))
        );
        assert!(forwarded.headers().iter().any(
            |header| header.field.equiv("via") && header.value.as_str().contains("1.1 txbase")
        ));
        forwarded
            .respond(
                Response::from_string("ok")
                    .with_header(Header::from_bytes("Connection", "x-response-secret").unwrap())
                    .with_header(Header::from_bytes("X-Response-Secret", "hidden").unwrap())
                    .with_header(Header::from_bytes("Via", "1.0 backend").unwrap()),
            )
            .unwrap();
    });

    assert!(!completes_tls_handshake(
        listener.public_addr(),
        Arc::clone(&certificates.missing_client_config)
    ));
    assert!(!completes_tls_handshake(
        listener.public_addr(),
        Arc::clone(&certificates.untrusted_client_config)
    ));

    let bypass = request(
        backend_addr,
        b"GET /private HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    assert!(String::from_utf8_lossy(&bypass).starts_with("HTTP/1.1 403"));

    let response = send_tls_request(
        listener.public_addr(),
        Arc::clone(&certificates.client_config),
        b"GET /catalog?mode=full HTTP/1.1\r\nHost: client.invalid\r\nConnection: x-hop-secret\r\nX-Hop-Secret: hidden\r\nX-Txbase-Internal-Listener-Token: attacker\r\nVia: 1.0 client\r\n\r\n",
    );
    let response = String::from_utf8_lossy(&response);
    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("1.0 backend"));
    assert!(response.contains("1.1 txbase"));
    assert!(!response.to_ascii_lowercase().contains("x-response-secret"));
    assert!(response.ends_with("ok"));

    backend_thread.join().unwrap();
}

#[test]
fn tls_config_rejects_missing_or_mismatched_files_before_catalog_open() {
    let certificates = test_certificates();
    let missing = CatalogTlsConfig {
        certificate: certificates.directory.join("missing.pem"),
        ..certificates.config.clone()
    };
    assert!(load_server_config(&missing).is_err());

    let mismatched_key = certificates.directory.join("other-key.pem");
    let other_key = generate_simple_self_signed(vec!["elsewhere".to_owned()]).unwrap();
    fs::write(&mismatched_key, other_key.key_pair.serialize_pem()).unwrap();
    let mismatched = CatalogTlsConfig {
        private_key: mismatched_key,
        ..certificates.config.clone()
    };
    assert!(load_server_config(&mismatched).is_err());
}

#[test]
fn server_tls_without_a_client_ca_accepts_clients_without_certificates() {
    let certificates = test_certificates();
    let config = CatalogTlsConfig {
        client_ca: None,
        ..certificates.config.clone()
    };
    let listener = CatalogListener::bind("127.0.0.1:0", Some(&config)).unwrap();
    assert!(completes_tls_handshake(
        listener.public_addr(),
        Arc::clone(&certificates.missing_client_config)
    ));
}

#[test]
fn internal_backend_tokens_are_random_and_unique() {
    let first = new_internal_token().unwrap();
    let second = new_internal_token().unwrap();
    assert_eq!(first.len(), 64);
    assert_ne!(first, second);
}
