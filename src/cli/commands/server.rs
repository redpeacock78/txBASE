use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;

use txbase::{dbf::DbfTable, server};

pub(crate) fn serve(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(args.next().ok_or("serve requires a DBF path")?);
    let mut bind = String::from("127.0.0.1:8080");
    let mut encoding = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--bind" => bind = args.next().ok_or("--bind requires an address")?,
            "--encoding" => encoding = Some(args.next().ok_or("--encoding requires a name")?),
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let table = DbfTable::from_path_with_encoding(&path, encoding.as_deref())?;
    server::serve(table, &path, &bind).map_err(Into::into)
}

pub(crate) fn serve_catalog(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        args.next()
            .ok_or("serve-catalog requires a directory path")?,
    );
    let mut bind = String::from("127.0.0.1:8080");
    let mut replication_term = 1;
    let mut replication_role = server::CatalogReplicationRole::Authority;
    let mut replication_options_used = false;
    let mut raft_node_id = None;
    let mut raft_cluster_id = None;
    let mut raft_node_directory = None;
    let mut raft_peer_bind = None;
    let mut raft_peer_advertise = None;
    let mut raft_initial_members = BTreeMap::new();
    let mut raft_bootstrap = false;
    let mut raft_initialize_catalog = false;
    let mut raft_tls_certificate = None;
    let mut raft_tls_private_key = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--bind" => bind = args.next().ok_or("--bind requires an address")?,
            "--replication-term" => {
                replication_options_used = true;
                replication_term = args
                    .next()
                    .ok_or("--replication-term requires a positive integer")?
                    .parse::<u64>()?;
                if replication_term == 0 {
                    return Err("--replication-term must be positive".into());
                }
            }
            "--replication-role" => {
                replication_options_used = true;
                replication_role = match args
                    .next()
                    .ok_or("--replication-role requires authority or follower")?
                    .as_str()
                {
                    "authority" => server::CatalogReplicationRole::Authority,
                    "follower" => server::CatalogReplicationRole::Follower,
                    role => return Err(format!("unsupported --replication-role: {role}").into()),
                };
            }
            "--raft-node-id" => {
                raft_node_id = Some(
                    args.next()
                        .ok_or("--raft-node-id requires a positive integer")?
                        .parse::<u64>()?,
                );
            }
            "--raft-cluster-id" => {
                raft_cluster_id = Some(
                    args.next()
                        .ok_or("--raft-cluster-id requires an identifier")?,
                );
            }
            "--raft-data-directory" => {
                raft_node_directory = Some(PathBuf::from(
                    args.next().ok_or("--raft-data-directory requires a path")?,
                ));
            }
            "--raft-peer-bind" => {
                raft_peer_bind = Some(args.next().ok_or("--raft-peer-bind requires an address")?);
            }
            "--raft-peer-advertise" => {
                raft_peer_advertise = Some(
                    args.next()
                        .ok_or("--raft-peer-advertise requires an HTTP(S) URL")?,
                );
            }
            "--raft-initial-member" => {
                let member = args
                    .next()
                    .ok_or("--raft-initial-member requires ID=HTTP(S)-URL")?;
                let (id, address) = member
                    .split_once('=')
                    .ok_or("--raft-initial-member requires ID=HTTP(S)-URL")?;
                let id = id.parse::<u64>()?;
                if raft_initial_members
                    .insert(id, address.to_owned())
                    .is_some()
                {
                    return Err(format!("duplicate Raft initial member ID: {id}").into());
                }
            }
            "--raft-bootstrap" => raft_bootstrap = true,
            "--raft-initialize-catalog" => raft_initialize_catalog = true,
            "--raft-peer-cert" => {
                raft_tls_certificate = Some(PathBuf::from(
                    args.next().ok_or("--raft-peer-cert requires a file path")?,
                ));
            }
            "--raft-peer-key" => {
                raft_tls_private_key = Some(PathBuf::from(
                    args.next().ok_or("--raft-peer-key requires a file path")?,
                ));
            }
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }
    let raft_options_used = raft_node_id.is_some()
        || raft_cluster_id.is_some()
        || raft_node_directory.is_some()
        || raft_peer_bind.is_some()
        || raft_peer_advertise.is_some()
        || !raft_initial_members.is_empty()
        || raft_bootstrap
        || raft_initialize_catalog
        || raft_tls_certificate.is_some()
        || raft_tls_private_key.is_some();
    if raft_options_used {
        if replication_options_used {
            return Err("--replication-* and --raft-* options cannot be combined".into());
        }
        return server::serve_catalog_with_raft(
            &path,
            &bind,
            server::CatalogRaftConfig {
                node_id: raft_node_id.ok_or("--raft-node-id is required for Raft mode")?,
                cluster_id: raft_cluster_id.ok_or("--raft-cluster-id is required for Raft mode")?,
                node_directory: raft_node_directory
                    .ok_or("--raft-data-directory is required for Raft mode")?,
                peer_bind: raft_peer_bind.ok_or("--raft-peer-bind is required for Raft mode")?,
                peer_advertise: raft_peer_advertise
                    .ok_or("--raft-peer-advertise is required for Raft mode")?,
                initial_members: raft_initial_members,
                bootstrap: raft_bootstrap,
                initialize_catalog: raft_initialize_catalog,
                tls_certificate: raft_tls_certificate,
                tls_private_key: raft_tls_private_key,
            },
        )
        .map_err(Into::into);
    }
    server::serve_catalog_with_replication_config(&path, &bind, replication_term, replication_role)
        .map_err(Into::into)
}
