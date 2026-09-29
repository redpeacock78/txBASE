use super::*;
use crate::server::catalog_tls::tests::{
    send_tls_request, test_certificates, tls_request_is_rejected,
};

#[test]
fn raft_peer_listener_requires_a_trusted_client_certificate() {
    let root = temporary_cluster();
    let catalog_root = root.join("catalog-1");
    prepare_catalog(&catalog_root);
    let certificates = test_certificates();
    let peer_bind = free_address();
    let peer_port = peer_bind.rsplit_once(':').unwrap().1;
    let peer_url = format!("https://localhost:{peer_port}");
    let config = CatalogRaftConfig {
        node_id: 1,
        cluster_id: "ci-raft-mtls".into(),
        node_directory: root.join("node-1"),
        peer_bind,
        peer_advertise: peer_url.clone(),
        initial_members: BTreeMap::from([(1, peer_url)]),
        bootstrap: true,
        initialize_catalog: false,
        tls_certificate: Some(certificates.config.certificate.clone()),
        tls_private_key: Some(certificates.config.private_key.clone()),
        tls_client_ca: certificates.config.client_ca.clone(),
    };
    let raft = RaftRuntime::start(&catalog_root, config.clone(), Some("ci-token".into())).unwrap();
    let peer_server = raft.bind_peer_listener(&config).unwrap();
    let address = peer_server.public_addr();
    let peer_listener = raft.spawn_peer_listener(peer_server).unwrap();
    let request = b"GET /raft/v1/membership HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer ci-token\r\nConnection: close\r\n\r\n";

    let response = send_tls_request(address, certificates.client_config.clone(), request);
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200"));
    assert!(tls_request_is_rejected(
        address,
        certificates.missing_client_config.clone(),
        request
    ));
    assert!(tls_request_is_rejected(
        address,
        certificates.untrusted_client_config.clone(),
        request
    ));

    drop(peer_listener);
    raft.shutdown().unwrap();
    drop(raft);
    fs::remove_dir_all(root).unwrap();
}
