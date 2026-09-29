use super::*;
use crate::replication::http_tests::{
    spawn_tls_handshake_probe, spawn_tls_status, tls_client_auth_config_pair, tls_config_pair,
};
use rustls::{ClientConfig, RootCertStore};

fn network_with_tls(url: &str, tls: Arc<ClientConfig>) -> RaftHttpNetwork {
    let factory =
        RaftHttpNetworkFactory::new("tls-cluster", 1, vec![7; 32], "peer-secret").unwrap();
    let mut network = factory.new_network(2, url);
    network.client = network
        .client
        .clone()
        .map(|client| client.with_tls_config_for_test(tls));
    network
}

#[test]
fn peer_rpc_uses_verified_https_and_rejects_untrusted_or_wrong_host_certificates() {
    let (trusted_client, server_tls) = tls_config_pair();
    let response: RpcReply<String, String> =
        reply("tls-cluster", 2, &[7; 32], Ok("accepted".into()));
    let (url, server) =
        spawn_tls_status(serde_json::to_vec(&response).unwrap(), server_tls.clone());
    let network = network_with_tls(&url, trusted_client.clone());
    let result = network
        .rpc::<(), String, String>(RAFT_VOTE_PATH, (), Duration::from_secs(2))
        .unwrap();
    assert_eq!(result.unwrap(), "accepted");

    let request = server.join().unwrap();
    assert!(request.starts_with("POST /api/raft/v1/vote HTTP/1.1\r\n"));
    assert!(request.contains("\r\nAuthorization: Bearer peer-secret\r\n"));
    let body_start = request.find("\r\n\r\n").unwrap() + 4;
    let wire: RpcRequestWire<()> = serde_json::from_str(&request[body_start..]).unwrap();
    assert_eq!(wire.cluster_id, "tls-cluster");
    assert_eq!(wire.sender_id, 1);
    assert_eq!(wire.genesis_fingerprint, vec![7; 32]);

    let untrusted_client = ClientConfig::builder()
        .with_root_certificates(RootCertStore::empty())
        .with_no_client_auth();
    let (url, server) = spawn_tls_handshake_probe(server_tls.clone(), "localhost");
    let network = network_with_tls(&url, Arc::new(untrusted_client));
    assert!(
        network
            .rpc::<(), String, String>(RAFT_VOTE_PATH, (), Duration::from_secs(2))
            .is_err()
    );
    assert!(server.join().unwrap());

    let (url, server) = spawn_tls_handshake_probe(server_tls, "127.0.0.1");
    let network = network_with_tls(&url, trusted_client);
    assert!(
        network
            .rpc::<(), String, String>(RAFT_VOTE_PATH, (), Duration::from_secs(2))
            .is_err()
    );
    assert!(server.join().unwrap());
}

#[test]
fn peer_rpc_presents_a_client_certificate_to_an_mtls_peer() {
    let (client_tls, _, server_tls) = tls_client_auth_config_pair();
    let response: RpcReply<String, String> =
        reply("tls-cluster", 2, &[7; 32], Ok("accepted".into()));
    let (url, server) = spawn_tls_status(serde_json::to_vec(&response).unwrap(), server_tls);
    let network = network_with_tls(&url, client_tls);

    let result = network
        .rpc::<(), String, String>(RAFT_VOTE_PATH, (), Duration::from_secs(2))
        .unwrap();

    assert_eq!(result.unwrap(), "accepted");
    assert!(
        server
            .join()
            .unwrap()
            .starts_with("POST /api/raft/v1/vote HTTP/1.1\r\n")
    );
}

#[test]
fn peer_rpc_without_a_client_certificate_is_rejected_by_an_mtls_peer() {
    let (_, anonymous_client_tls, server_tls) = tls_client_auth_config_pair();
    let (url, server) = spawn_tls_handshake_probe(server_tls, "localhost");
    let network = network_with_tls(&url, anonymous_client_tls);

    assert!(
        network
            .rpc::<(), String, String>(RAFT_VOTE_PATH, (), Duration::from_secs(2))
            .is_err()
    );
    assert!(server.join().unwrap());
}
