use super::CatalogRaftConfig;
use crate::replication::ReplicationHttpClient;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

const NODE_IDENTITY_NAME: &str = ".txbase-raft-node";

pub(super) fn validate_config(
    catalog_root: &Path,
    config: &CatalogRaftConfig,
    token: &str,
) -> Result<bool, String> {
    if config.node_id == 0 {
        return Err("Raft node ID must be positive".into());
    }
    if config.cluster_id.is_empty()
        || config.cluster_id.len() > 128
        || !config
            .cluster_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(
            "Raft cluster ID must be 1 to 128 ASCII letters, digits, '.', '_', or '-'".into(),
        );
    }
    if config.initial_members.is_empty() {
        return Err("Raft initial membership must include this node".into());
    }
    let self_address = config
        .initial_members
        .get(&config.node_id)
        .ok_or_else(|| "Raft initial membership must include this node ID".to_owned())?;
    if self_address != &config.peer_advertise {
        return Err("this node's initial-member URL must match --raft-peer-advertise".into());
    }
    if config.initial_members.keys().any(|id| *id == 0) {
        return Err("Raft initial member IDs must be positive".into());
    }
    let mut addresses = BTreeSet::new();
    for address in config.initial_members.values() {
        if !addresses.insert(address) {
            return Err("Raft initial members must use distinct peer URLs".into());
        }
        ReplicationHttpClient::new(address)
            .and_then(|client| client.with_bearer_token(token.to_owned()))
            .map_err(|error| format!("invalid Raft peer URL {address}: {error}"))?;
    }
    if config.tls_certificate.is_some() != config.tls_private_key.is_some() {
        return Err("Raft peer TLS requires both a certificate and private key".into());
    }
    let peer_tls = config.peer_advertise.starts_with("https://");
    if peer_tls != config.tls_certificate.is_some() {
        return Err("Raft peer TLS files must match the advertised URL scheme".into());
    }
    if !config.peer_advertise.starts_with("http://") && !peer_tls {
        return Err("Raft peer advertise URL must use HTTP or HTTPS".into());
    }
    let catalog_root = fs::canonicalize(catalog_root)
        .map_err(|error| format!("cannot resolve catalog directory: {error}"))?;
    fs::create_dir_all(&config.node_directory)
        .map_err(|error| format!("cannot create Raft node directory: {error}"))?;
    let node_directory = fs::canonicalize(&config.node_directory)
        .map_err(|error| format!("cannot resolve Raft node directory: {error}"))?;
    if node_directory.starts_with(&catalog_root) || catalog_root.starts_with(&node_directory) {
        return Err("Raft node directory and catalog directory must be separate".into());
    }
    Ok(peer_tls)
}

pub(super) fn bind_node_identity(
    path: &Path,
    node_id: u64,
    cluster_id: &str,
) -> Result<(), String> {
    let marker = path.join(NODE_IDENTITY_NAME);
    let expected = format!("TXRN1\n{cluster_id}\n{node_id}\n");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(mut file) => {
            file.write_all(expected.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|error| format!("cannot persist Raft node identity: {error}"))?;
            File::open(path)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("cannot sync Raft node directory: {error}"))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let actual = fs::read_to_string(&marker)
                .map_err(|error| format!("cannot read Raft node identity: {error}"))?;
            if actual == expected {
                Ok(())
            } else {
                Err("Raft node directory belongs to a different node or cluster".into())
            }
        }
        Err(error) => Err(format!("cannot create Raft node identity: {error}")),
    }
}

pub(super) fn read_tls_file(path: Option<&Path>) -> Result<Vec<u8>, String> {
    let path = path.ok_or_else(|| "Raft TLS file path is missing".to_owned())?;
    fs::read(path).map_err(|error| format!("cannot read Raft TLS file {}: {error}", path.display()))
}
