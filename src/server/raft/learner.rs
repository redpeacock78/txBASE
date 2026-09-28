use super::{RPC_TIMEOUT, RaftRuntime, SNAPSHOT_TIMEOUT, config};
use crate::replication::raft::TypeConfig;
use openraft::{BasicNode, Snapshot, Vote};
use std::time::Duration;

impl RaftRuntime {
    pub(super) fn add_learner(
        &self,
        node_id: u64,
        peer_address: &str,
    ) -> Result<bool, (u16, String)> {
        if node_id == 0 || node_id == self.node_id {
            return Err((
                400,
                "learner node ID must be positive and different from this node".into(),
            ));
        }
        config::validate_peer_url(peer_address, &self.token).map_err(|error| (422, error))?;
        let (existing_address, learner_url_in_use, is_leader) = {
            let metrics = self.node.metrics();
            let metrics = metrics.borrow();
            let membership = &metrics.membership_config;
            (
                membership
                    .nodes()
                    .find(|(id, _)| **id == node_id)
                    .map(|(_, node)| node.addr.clone()),
                membership
                    .nodes()
                    .any(|(_, node)| node.addr == peer_address),
                metrics.current_leader == Some(self.node_id),
            )
        };
        if let Some(existing_address) = existing_address {
            if existing_address == peer_address {
                return Ok(false);
            }
            return Err((
                409,
                format!("Raft node ID {node_id} is already in the membership"),
            ));
        }
        if learner_url_in_use {
            return Err((409, "Raft learner URL is already in the membership".into()));
        }
        if !is_leader {
            return Err((
                409,
                "learner additions must be sent to the Raft leader".into(),
            ));
        }
        let guard = self
            .membership_change_lock
            .clone()
            .try_lock_owned()
            .map_err(|_| (409, "another membership change is in progress".into()))?;
        let runtime = self.clone();
        let learner = BasicNode::new(peer_address);
        self.runtime.spawn(async move {
            let _guard = guard;
            if let Err((_, error)) = runtime.add_learner_locked(node_id, learner, false).await {
                eprintln!(
                    "Raft learner {node_id} failed on node {}: {error}",
                    runtime.node_id
                );
            }
        });
        Ok(true)
    }

    pub(super) async fn add_learner_locked(
        &self,
        node_id: u64,
        learner: BasicNode,
        blocking: bool,
    ) -> Result<bool, (u16, String)> {
        let (existing_address, learner_url_in_use, is_leader) = {
            let metrics = self.node.metrics();
            let metrics = metrics.borrow();
            let membership = &metrics.membership_config;
            (
                membership
                    .nodes()
                    .find(|(id, _)| **id == node_id)
                    .map(|(_, node)| node.addr.clone()),
                membership
                    .nodes()
                    .any(|(_, node)| node.addr == learner.addr),
                metrics.current_leader == Some(self.node_id),
            )
        };
        if let Some(existing_address) = existing_address {
            if existing_address != learner.addr {
                return Err((
                    409,
                    format!("Raft node ID {node_id} is already in the membership"),
                ));
            }
            if blocking {
                self.node
                    .add_learner(node_id, learner, true)
                    .await
                    .map_err(|error| (503, format!("cannot catch up Raft learner: {error}")))?;
            }
            return Ok(false);
        }
        if learner_url_in_use {
            return Err((409, "Raft learner URL is already in the membership".into()));
        }
        if !is_leader {
            return Err((
                409,
                "learner additions must be sent to the Raft leader".into(),
            ));
        }

        let network = self.network_factory.clone();
        let address = learner.addr.clone();
        tokio::task::spawn_blocking(move || {
            network.prepare_learner(node_id, &address, RPC_TIMEOUT)
        })
        .await
        .map_err(|error| (503, format!("learner preparation task failed: {error}")))??;

        let (vote, snapshot) = self
            .snapshot_for_learner()
            .await
            .map_err(|error| (503, error))?;
        let network = self.network_factory.clone();
        let address = learner.addr.clone();
        tokio::task::spawn_blocking(move || {
            network.transfer_snapshot(node_id, &address, vote, snapshot, RPC_TIMEOUT)
        })
        .await
        .map_err(|error| (503, format!("learner snapshot task failed: {error}")))?
        .map_err(|error| (503, error))?;

        if blocking {
            self.node
                .add_learner(node_id, learner, true)
                .await
                .map(|_| true)
                .map_err(|error| (503, format!("cannot add Raft learner: {error}")))
        } else {
            tokio::time::timeout(RPC_TIMEOUT, self.node.add_learner(node_id, learner, false))
                .await
                .map_err(|_| (504, "Raft learner addition timed out".to_owned()))?
                .map(|_| true)
                .map_err(|error| (503, format!("cannot add Raft learner: {error}")))
        }
    }

    async fn snapshot_for_learner(&self) -> Result<(Vote<u64>, Snapshot<TypeConfig>), String> {
        let target_index = {
            let metrics = self.node.metrics();
            let metrics = metrics.borrow();
            if metrics.current_leader != Some(self.node_id) {
                return Err("Raft leadership changed before snapshot transfer".into());
            }
            metrics
                .last_applied
                .as_ref()
                .map(|log_id| log_id.index)
                .ok_or_else(|| "Raft leader has no applied log to snapshot".to_owned())?
        };

        let snapshot_ready = {
            let metrics = self.node.metrics();
            metrics
                .borrow()
                .snapshot
                .as_ref()
                .is_some_and(|log_id| log_id.index >= target_index)
        };
        if !snapshot_ready {
            self.node
                .trigger()
                .snapshot()
                .await
                .map_err(|error| error.to_string())?;
        }

        let snapshot = tokio::time::timeout(SNAPSHOT_TIMEOUT, async {
            loop {
                let ready = {
                    let metrics = self.node.metrics();
                    metrics
                        .borrow()
                        .snapshot
                        .as_ref()
                        .is_some_and(|log_id| log_id.index >= target_index)
                };
                if ready {
                    if let Some(snapshot) = self
                        .node
                        .get_snapshot()
                        .await
                        .map_err(|error| error.to_string())?
                    {
                        if snapshot
                            .meta
                            .last_log_id
                            .as_ref()
                            .is_some_and(|log_id| log_id.index >= target_index)
                        {
                            return Ok::<_, String>(snapshot);
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| "timed out building a Raft learner snapshot".to_owned())??;

        let metrics = self.node.metrics();
        let metrics = metrics.borrow();
        if metrics.current_leader != Some(self.node_id) {
            return Err("Raft leadership changed before learner snapshot transfer".into());
        }
        Ok((metrics.vote, snapshot))
    }
}
