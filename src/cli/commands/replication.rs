use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::time::Duration;

use txbase::catalog::Catalog;
use txbase::replication::{
    MAX_REPLICATION_ENTRY_BATCH, ReplicationHttpClient, ReplicationLog, ReplicationProgress,
};

pub(crate) fn replicate(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    match args.next().as_deref() {
        Some("catch-up") => catch_up(args),
        Some(action) => Err(format!("unknown replicate command: {action}").into()),
        None => Err("replicate requires catch-up".into()),
    }
}

fn catch_up(mut args: impl Iterator<Item = String>) -> Result<(), Box<dyn Error>> {
    let directory = PathBuf::from(
        args.next()
            .ok_or("replicate catch-up requires a catalog directory")?,
    );
    let authority_url = args
        .next()
        .ok_or("replicate catch-up requires an authority URL")?;
    let mut term = None;
    let mut follower_id = None;
    let mut limit = MAX_REPLICATION_ENTRY_BATCH;
    let mut timeout = Duration::from_secs(10);
    while let Some(option) = args.next() {
        match option.as_str() {
            "--replication-term" => {
                term = Some(parse_positive_u64(
                    args.next()
                        .ok_or("--replication-term requires a positive integer")?,
                    "replication term",
                )?);
            }
            "--follower-id" => {
                follower_id = Some(args.next().ok_or("--follower-id requires an identifier")?);
            }
            "--limit" => {
                let value = parse_positive_usize(
                    args.next().ok_or("--limit requires a positive integer")?,
                    "replication entry limit",
                )?;
                if value > MAX_REPLICATION_ENTRY_BATCH {
                    return Err(format!(
                        "replication entry limit must be at most {MAX_REPLICATION_ENTRY_BATCH}"
                    )
                    .into());
                }
                limit = value;
            }
            "--timeout-ms" => {
                timeout = Duration::from_millis(parse_positive_u64(
                    args.next()
                        .ok_or("--timeout-ms requires a positive integer")?,
                    "replication timeout",
                )?);
            }
            _ => return Err(format!("unknown option: {option}").into()),
        }
    }

    let term = term.ok_or("replicate catch-up requires --replication-term TERM")?;
    let follower_id = follower_id.ok_or("replicate catch-up requires --follower-id ID")?;
    ReplicationProgress::new(follower_id.clone(), term, 0, 0, "cli".into())?;
    let mut catalog = Catalog::from_path(&directory)?;
    let mut log = ReplicationLog::open(&catalog, term)?;
    let mut client = ReplicationHttpClient::new(&authority_url)?.with_timeout(timeout)?;
    if let Some(token) = env::var_os("TXBASE_REPLICATION_TOKEN") {
        let token = token
            .into_string()
            .map_err(|_| "TXBASE_REPLICATION_TOKEN must be valid UTF-8")?;
        client = client.with_bearer_token(token)?;
    }
    let result = client.catch_up(&mut catalog, &mut log, follower_id, limit)?;
    println!(
        "{}",
        serde_json::json!({
            "snapshot_installed": result.snapshot_installed,
            "entries_applied": result.entries_applied,
            "progress": result.progress,
        })
    );
    Ok(())
}

fn parse_positive_u64(value: String, label: &str) -> Result<u64, Box<dyn Error>> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| format!("{label} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{label} must be a positive integer").into());
    }
    Ok(parsed)
}

fn parse_positive_usize(value: String, label: &str) -> Result<usize, Box<dyn Error>> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("{label} must be a positive integer"))?;
    if parsed == 0 {
        return Err(format!("{label} must be a positive integer").into());
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::{parse_positive_u64, parse_positive_usize, replicate};

    #[test]
    fn positive_replication_options_reject_zero_and_non_numeric_values() {
        assert_eq!(parse_positive_u64("4".into(), "term").unwrap(), 4);
        assert_eq!(parse_positive_usize("8".into(), "limit").unwrap(), 8);
        assert!(parse_positive_u64("0".into(), "term").is_err());
        assert!(parse_positive_usize("nope".into(), "limit").is_err());
    }

    #[test]
    fn replicate_rejects_missing_or_unknown_subcommands() {
        assert!(replicate(std::iter::empty()).is_err());
        assert!(replicate([String::from("status")].into_iter()).is_err());
    }
}
