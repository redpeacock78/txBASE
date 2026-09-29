use super::*;
use crate::catalog::Catalog;
use crate::xbase::{OperationIr, OperationMethod};
use rcgen::{
    BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, KeyPair,
    generate_simple_self_signed,
};
use rustls::pki_types::PrivateKeyDer;
use rustls::{ClientConfig, RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use serde_json::json;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

static NEXT_HTTP_TEST_ID: AtomicUsize = AtomicUsize::new(0);

fn temporary_catalog(label: &str) -> PathBuf {
    let id = NEXT_HTTP_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-replication-http-{label}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    let fixture = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    fs::write(root.join("users.dbf"), fixture).unwrap();
    root
}

fn post(record_id: i64, name: &str) -> OperationIr {
    OperationIr {
        method: OperationMethod::Post,
        path: "/users/records".into(),
        body: Some(json!({
            "ID": record_id,
            "NAME": name,
            "AGE": 42,
            "ACTIVE": true
        })),
    }
}

fn response(body: Vec<u8>) -> Vec<u8> {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body)
    .collect()
}

fn error_response(status: u16, code: &str) -> Vec<u8> {
    let body = json!({
        "error": {
            "code": code,
            "message": "temporary replication failure"
        }
    })
    .to_string()
    .into_bytes();
    format!(
        "HTTP/1.1 {status} Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body)
    .collect()
}

fn spawn_sequence(responses: Vec<Vec<u8>>) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            requests.push(String::from_utf8(request).unwrap());
            stream.write_all(&response).unwrap();
        }
        requests
    });
    (format!("http://{address}/api/"), handle)
}

fn read_request(stream: &mut impl Read) -> Vec<u8> {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0);
        request.extend_from_slice(&buffer[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&request[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while request.len() < header_end + content_length {
        let read = stream.read(&mut buffer).unwrap();
        assert!(read > 0);
        request.extend_from_slice(&buffer[..read]);
    }
    request
}

pub(in crate::replication) fn tls_config_pair() -> (Arc<ClientConfig>, Arc<ServerConfig>) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate = cert.der().clone();
    let mut roots = RootCertStore::empty();
    roots.add(certificate.clone()).unwrap();

    let client = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let server = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![certificate],
            PrivateKeyDer::Pkcs8(key_pair.serialize_der().into()),
        )
        .unwrap();
    (Arc::new(client), Arc::new(server))
}

pub(in crate::replication) fn tls_client_auth_config_pair()
-> (Arc<ClientConfig>, Arc<ClientConfig>, Arc<ServerConfig>) {
    let server = generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let mut server_roots = RootCertStore::empty();
    server_roots.add(server.cert.der().clone()).unwrap();

    let mut client_ca_params = CertificateParams::default();
    client_ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let client_ca_key = KeyPair::generate().unwrap();
    let client_ca = client_ca_params.self_signed(&client_ca_key).unwrap();
    let mut client_params = CertificateParams::default();
    client_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let client_key = KeyPair::generate().unwrap();
    let client = client_params
        .signed_by(&client_key, &client_ca, &client_ca_key)
        .unwrap();

    let anonymous_client_config = ClientConfig::builder()
        .with_root_certificates(server_roots.clone())
        .with_no_client_auth();
    let client_config = ClientConfig::builder()
        .with_root_certificates(server_roots)
        .with_client_auth_cert(
            vec![client.der().clone()],
            PrivateKeyDer::Pkcs8(client_key.serialize_der().into()),
        )
        .unwrap();
    let mut client_roots = RootCertStore::empty();
    client_roots.add(client_ca.der().clone()).unwrap();
    let verifier = rustls::server::WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .unwrap();
    let server_config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(
            vec![server.cert.der().clone()],
            PrivateKeyDer::Pkcs8(server.key_pair.serialize_der().into()),
        )
        .unwrap();
    (
        Arc::new(client_config),
        Arc::new(anonymous_client_config),
        Arc::new(server_config),
    )
}

fn spawn_tls_sequence(
    responses: Vec<Vec<u8>>,
    config: Arc<ServerConfig>,
) -> (String, JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let (socket, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(Arc::clone(&config)).unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            requests.push(String::from_utf8(read_request(&mut stream)).unwrap());
            stream.write_all(&response).unwrap();
            stream.conn.send_close_notify();
            stream.flush().unwrap();
        }
        requests
    });
    (format!("https://localhost:{port}/api/"), handle)
}

pub(in crate::replication) fn spawn_tls_status(
    body: Vec<u8>,
    config: Arc<ServerConfig>,
) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(config).unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let request = String::from_utf8(read_request(&mut stream)).unwrap();
        stream.write_all(&response(body)).unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
        request
    });
    (format!("https://localhost:{port}/api/"), handle)
}

pub(in crate::replication) fn spawn_tls_handshake_probe(
    config: Arc<ServerConfig>,
    host: &str,
) -> (String, JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut connection = ServerConnection::new(config).unwrap();
        connection.complete_io(&mut socket).is_err()
    });
    (format!("https://{host}:{port}/api/"), handle)
}

#[test]
fn client_validates_plain_http_configuration() {
    assert!(matches!(
        ReplicationHttpClient::new("ftp://127.0.0.1"),
        Err(ReplicationHttpError::UnsupportedScheme(scheme)) if scheme == "ftp"
    ));
    assert!(ReplicationHttpClient::new("https://127.0.0.1").is_ok());
    assert!(matches!(
        ReplicationHttpClient::new("http://127.0.0.1?token=secret"),
        Err(ReplicationHttpError::InvalidUrl(_))
    ));
    assert!(matches!(
        ReplicationHttpClient::new("http://127.0.0.1")
            .unwrap()
            .with_timeout(Duration::ZERO),
        Err(ReplicationHttpError::InvalidConfig(_))
    ));
    assert!(matches!(
        ReplicationHttpClient::new("http://replica.example.com")
            .unwrap()
            .with_bearer_token("secret"),
        Err(ReplicationHttpError::InvalidConfig(_))
    ));
    assert!(matches!(
        ReplicationHttpClient::new("http://127.0.0.1")
            .unwrap()
            .with_bearer_token("secret\r\nX-Injected: yes"),
        Err(ReplicationHttpError::InvalidConfig(_))
    ));
    assert!(matches!(
        ReplicationRetryPolicy::new(0, Duration::ZERO, Duration::ZERO),
        Err(ReplicationHttpError::InvalidConfig(_))
    ));
    assert!(matches!(
        ReplicationRetryPolicy::new(2, Duration::from_secs(2), Duration::from_secs(1)),
        Err(ReplicationHttpError::InvalidConfig(_))
    ));
}

#[test]
fn client_sends_authenticated_status_over_verified_https() {
    let (client_tls, server_tls) = tls_config_pair();
    let status = json!({
        "transport_version": REPLICATION_TRANSPORT_VERSION,
        "role": "authority",
        "term": 4,
        "base_index": 0,
        "base_transaction_id": 0,
        "last_index": 0,
        "last_transaction_id": 0,
        "follower_count": 0,
        "safe_compaction_index": null,
        "schema_tag": "schema-v1"
    });
    let (url, server) = spawn_tls_status(status.to_string().into_bytes(), server_tls);
    let client = ReplicationHttpClient::new(&url)
        .unwrap()
        .with_tls_config_for_test(client_tls)
        .with_bearer_token("secret")
        .unwrap();

    assert_eq!(client.status().unwrap().term, 4);
    let request = server.join().unwrap();
    assert!(request.starts_with("GET /api/replication/status HTTP/1.1\r\n"));
    assert!(request.contains("\r\nAuthorization: Bearer secret\r\n"));
}

#[test]
fn client_rejects_untrusted_https_certificate_without_retrying_as_http() {
    let (_, server_tls) = tls_config_pair();
    let (url, server) = spawn_tls_handshake_probe(server_tls, "localhost");
    let client = ReplicationHttpClient::new(&url)
        .unwrap()
        .with_bearer_token("secret")
        .unwrap();

    assert!(matches!(client.status(), Err(ReplicationHttpError::Tls(_))));
    assert!(server.join().unwrap());
}

#[test]
fn client_reads_status_with_base_path_and_bearer_auth() {
    let status = json!({
        "transport_version": REPLICATION_TRANSPORT_VERSION,
        "role": "authority",
        "term": 4,
        "base_index": 0,
        "base_transaction_id": 0,
        "last_index": 0,
        "last_transaction_id": 0,
        "follower_count": 0,
        "safe_compaction_index": null,
        "schema_tag": "schema-v1"
    });
    let (url, server) = spawn_sequence(vec![response(status.to_string().into_bytes())]);
    let client = ReplicationHttpClient::new(&url)
        .unwrap()
        .with_bearer_token("secret")
        .unwrap();
    let received = client.status().unwrap();
    let requests = server.join().unwrap();

    assert_eq!(received.term, 4);
    assert!(requests[0].starts_with("GET /api/replication/status HTTP/1.1\r\n"));
    assert!(requests[0].contains("\r\nAuthorization: Bearer secret\r\n"));
}

#[test]
fn client_retries_transient_replication_status() {
    let status = json!({
        "transport_version": REPLICATION_TRANSPORT_VERSION,
        "role": "authority",
        "term": 4,
        "base_index": 0,
        "base_transaction_id": 0,
        "last_index": 0,
        "last_transaction_id": 0,
        "follower_count": 0,
        "safe_compaction_index": null,
        "schema_tag": "schema-v1"
    });
    let (url, server) = spawn_sequence(vec![
        error_response(503, "temporarily_unavailable"),
        response(status.to_string().into_bytes()),
    ]);
    let client = ReplicationHttpClient::new(&url)
        .unwrap()
        .with_retry_policy(ReplicationRetryPolicy::new(2, Duration::ZERO, Duration::ZERO).unwrap());

    assert_eq!(client.status().unwrap().term, 4);
    assert_eq!(server.join().unwrap().len(), 2);
}

#[test]
fn client_does_not_retry_terminal_replication_status() {
    let (url, server) = spawn_sequence(vec![error_response(409, "conflict")]);
    let client = ReplicationHttpClient::new(&url)
        .unwrap()
        .with_retry_policy(ReplicationRetryPolicy::new(2, Duration::ZERO, Duration::ZERO).unwrap());

    assert!(matches!(
        client.status(),
        Err(ReplicationHttpError::HttpStatus {
            status: 409,
            code: Some(code),
            ..
        }) if code == "conflict"
    ));
    assert_eq!(server.join().unwrap().len(), 1);
}

#[test]
fn client_catches_up_over_https_with_a_client_certificate() {
    let leader_root = temporary_catalog("leader");
    let follower_root = temporary_catalog("follower");
    let leader = Catalog::from_path(&leader_root).unwrap();
    let mut follower = Catalog::from_path(&follower_root).unwrap();
    let (_, schema_tag) = leader.schema_representation().unwrap();
    let mut authority = ReplicationLog::new(4).unwrap();
    let entry = authority.propose(&leader, vec![post(3, "Carol")]).unwrap();
    let batch = authority.entry_batch(0, 1).unwrap();

    let responses = vec![
        response(
            json!({
                "transport_version": REPLICATION_TRANSPORT_VERSION,
                "role": "authority",
                "term": 4,
                "base_index": 0,
                "base_transaction_id": 0,
                "last_index": 1,
                "last_transaction_id": 1,
                "follower_count": 0,
                "safe_compaction_index": null,
                "schema_tag": schema_tag
            })
            .to_string()
            .into_bytes(),
        ),
        response(batch.to_json().unwrap()),
        response(
            json!({
                "transport_version": REPLICATION_TRANSPORT_VERSION,
                "outcome": "accepted",
                "follower_id": "follower-1",
                "index": 1,
                "transaction_id": 1,
                "safe_compaction_index": 1
            })
            .to_string()
            .into_bytes(),
        ),
    ];
    let (client_tls, _, server_tls) = tls_client_auth_config_pair();
    let (url, server) = spawn_tls_sequence(responses, server_tls);
    let client = ReplicationHttpClient::new(&url)
        .unwrap()
        .with_tls_config_for_test(client_tls)
        .with_bearer_token("secret")
        .unwrap();
    let mut replica = ReplicationLog::new(4).unwrap();
    let result = client
        .catch_up(&mut follower, &mut replica, "follower-1".into(), 1)
        .unwrap();
    let requests = server.join().unwrap();

    assert_eq!(entry.index, 1);
    assert_eq!(result.entries_applied, 1);
    assert!(!result.snapshot_installed);
    assert_eq!(replica.last_index(), 1);
    assert_eq!(follower.transaction_id().unwrap(), Some(1));
    assert!(requests[1].starts_with("GET /api/replication/entries?after=0&limit=1 HTTP/1.1\r\n"));
    assert!(requests[2].starts_with("POST /api/replication/progress HTTP/1.1\r\n"));
    assert!(requests[2].contains("\"follower_id\":\"follower-1\""));
    assert!(
        requests
            .iter()
            .all(|request| request.contains("Bearer secret"))
    );

    let _ = fs::remove_dir_all(leader_root);
    let _ = fs::remove_dir_all(follower_root);
}
