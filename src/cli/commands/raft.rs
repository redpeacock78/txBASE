use std::env;
use std::error::Error;
use std::time::Duration;

use txbase::replication::raft::{
    RaftAddLearnerRequest, RaftMembershipChangeRequest, RaftMembershipHttpClient,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) fn raft(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match args.next().as_deref() {
        Some("membership") => membership(args),
        Some(command) => Err(format!("unknown raft command: {command}").into()),
        None => Err("raft requires membership".into()),
    }
}

fn membership(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match args.next().as_deref() {
        Some("status") => status(args),
        Some("add-learner") => add_learner(args),
        Some("change-voters") => change_voters(args),
        Some(command) => Err(format!("unknown raft membership command: {command}").into()),
        None => Err("raft membership requires status, add-learner, or change-voters".into()),
    }
}

fn status(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let peer_url = args
        .next()
        .ok_or("raft membership status requires a peer URL")?;
    let timeout = parse_timeout_options(&mut args)?;
    let response = client(&peer_url, timeout)?.status()?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

fn add_learner(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let peer_url = args
        .next()
        .ok_or("raft membership add-learner requires a peer URL")?;
    let mut cluster_id = None;
    let mut node_id = None;
    let mut peer_address = None;
    let mut timeout = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--cluster-id" => set_once(
                &mut cluster_id,
                args.next().ok_or("--cluster-id requires an ID")?,
                "--cluster-id",
            )?,
            "--node-id" => set_once(
                &mut node_id,
                parse_positive_u64(
                    args.next().ok_or("--node-id requires a positive integer")?,
                    "node ID",
                )?,
                "--node-id",
            )?,
            "--peer-address" => set_once(
                &mut peer_address,
                args.next().ok_or("--peer-address requires a URL")?,
                "--peer-address",
            )?,
            "--timeout-ms" => set_once(
                &mut timeout,
                parse_timeout_ms(
                    args.next()
                        .ok_or("--timeout-ms requires a positive integer")?,
                )?,
                "--timeout-ms",
            )?,
            _ => return Err(format!("unknown raft membership option: {option}").into()),
        }
    }

    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let request = RaftAddLearnerRequest::new(
        cluster_id.ok_or("--cluster-id is required")?,
        node_id.ok_or("--node-id is required")?,
        peer_address.ok_or("--peer-address is required")?,
    )?;
    let response = client(&peer_url, timeout)?.add_learner(&request)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

fn change_voters(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let peer_url = args
        .next()
        .ok_or("raft membership change-voters requires a peer URL")?;
    let mut cluster_id = None;
    let mut expected_index = None;
    let mut expected_voter_ids = None;
    let mut voter_ids = None;
    let mut timeout = None;
    while let Some(option) = args.next() {
        match option.as_str() {
            "--cluster-id" => set_once(
                &mut cluster_id,
                args.next().ok_or("--cluster-id requires an ID")?,
                "--cluster-id",
            )?,
            "--expected-index" => set_once(
                &mut expected_index,
                parse_u64(
                    args.next()
                        .ok_or("--expected-index requires a non-negative integer")?,
                    "expected membership index",
                )?,
                "--expected-index",
            )?,
            "--expected-voter-ids" => set_once(
                &mut expected_voter_ids,
                parse_node_ids(
                    &args.next().ok_or("--expected-voter-ids requires IDs")?,
                    "expected voter IDs",
                )?,
                "--expected-voter-ids",
            )?,
            "--voter-ids" => set_once(
                &mut voter_ids,
                parse_node_ids(&args.next().ok_or("--voter-ids requires IDs")?, "voter IDs")?,
                "--voter-ids",
            )?,
            "--timeout-ms" => set_once(
                &mut timeout,
                parse_timeout_ms(
                    args.next()
                        .ok_or("--timeout-ms requires a positive integer")?,
                )?,
                "--timeout-ms",
            )?,
            _ => return Err(format!("unknown raft membership option: {option}").into()),
        }
    }

    let timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
    let request = RaftMembershipChangeRequest::new(
        cluster_id.ok_or("--cluster-id is required")?,
        expected_index.ok_or("--expected-index is required")?,
        expected_voter_ids.ok_or("--expected-voter-ids is required")?,
        voter_ids.ok_or("--voter-ids is required")?,
    )?;
    let response = client(&peer_url, timeout)?.change_membership(&request)?;
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

fn parse_timeout_options(
    args: &mut impl Iterator<Item = String>,
) -> Result<Duration, Box<dyn Error>> {
    let mut timeout = None;
    while let Some(option) = args.next() {
        if option != "--timeout-ms" {
            return Err(format!("unknown raft membership option: {option}").into());
        }
        set_once(
            &mut timeout,
            parse_timeout_ms(
                args.next()
                    .ok_or("--timeout-ms requires a positive integer")?,
            )?,
            "--timeout-ms",
        )?;
    }
    Ok(timeout.unwrap_or(DEFAULT_TIMEOUT))
}

fn client(peer_url: &str, timeout: Duration) -> Result<RaftMembershipHttpClient, Box<dyn Error>> {
    let token = env::var("TXBASE_REPLICATION_TOKEN")
        .map_err(|_| "Raft membership commands require TXBASE_REPLICATION_TOKEN")?;
    Ok(RaftMembershipHttpClient::new(peer_url, token)?.with_timeout(timeout)?)
}

fn set_once<T>(slot: &mut Option<T>, value: T, option: &str) -> Result<(), Box<dyn Error>> {
    if slot.is_some() {
        return Err(format!("{option} may be specified only once").into());
    }
    *slot = Some(value);
    Ok(())
}

fn parse_node_ids(value: &str, label: &str) -> Result<Vec<u64>, Box<dyn Error>> {
    let ids = value
        .split(',')
        .map(|part| parse_positive_u64(part.to_owned(), label))
        .collect::<Result<Vec<_>, _>>()?;
    if ids.is_empty()
        || ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != ids.len()
    {
        return Err(format!("{label} must contain unique positive IDs").into());
    }
    Ok(ids)
}

fn parse_positive_u64(value: String, label: &str) -> Result<u64, Box<dyn Error>> {
    let parsed = parse_u64(value, label)?;
    if parsed == 0 {
        return Err(format!("{label} must be positive").into());
    }
    Ok(parsed)
}

fn parse_u64(value: String, label: &str) -> Result<u64, Box<dyn Error>> {
    value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be an unsigned integer").into())
}

fn parse_timeout_ms(value: String) -> Result<Duration, Box<dyn Error>> {
    Ok(Duration::from_millis(parse_positive_u64(value, "timeout")?))
}

#[cfg(test)]
mod tests {
    use super::{add_learner, parse_node_ids, raft};

    #[test]
    fn voter_id_lists_require_unique_positive_integers() {
        assert_eq!(parse_node_ids("1,2,30", "voter IDs").unwrap(), [1, 2, 30]);
        assert!(parse_node_ids("", "voter IDs").is_err());
        assert!(parse_node_ids("1,1", "voter IDs").is_err());
        assert!(parse_node_ids("0,2", "voter IDs").is_err());
        assert!(parse_node_ids("1,nope", "voter IDs").is_err());
    }

    #[test]
    fn raft_membership_cli_rejects_missing_and_unknown_commands_before_connecting() {
        assert!(raft(std::iter::empty()).is_err());
        assert!(raft(["membership".to_owned()].into_iter()).is_err());
        assert!(raft(["unknown".to_owned()].into_iter()).is_err());
        assert!(raft(["membership".to_owned(), "status".to_owned()].into_iter()).is_err());

        let error = raft(
            ["membership", "add-learner", "http://127.0.0.1"]
                .map(str::to_owned)
                .into_iter(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("--cluster-id is required"));

        let error = raft(
            ["membership", "change-voters", "http://127.0.0.1"]
                .map(str::to_owned)
                .into_iter(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("--cluster-id is required"));
    }

    #[test]
    fn raft_membership_cli_rejects_duplicate_options_before_connecting() {
        let error = add_learner(
            [
                "http://127.0.0.1",
                "--cluster-id",
                "cluster-a",
                "--cluster-id",
                "cluster-b",
            ]
            .map(str::to_owned)
            .into_iter(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("--cluster-id may be specified only once")
        );

        let error = raft(
            [
                "membership",
                "status",
                "http://127.0.0.1",
                "--timeout-ms",
                "10",
                "--timeout-ms",
                "20",
            ]
            .map(str::to_owned)
            .into_iter(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("--timeout-ms may be specified only once")
        );
    }
}
