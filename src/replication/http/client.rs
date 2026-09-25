use super::protocol::{
    ReplicationDeliveryResponse, ReplicationHttpError, ReplicationHttpStatus,
    ReplicationProgressResponse, ReplicationSyncResult, is_valid_bearer_token, parse_base_url,
    parse_json, read_response,
};
use crate::catalog::Catalog;
use crate::replication::{
    ApplyOutcome, MAX_REPLICATION_ENTRY_BATCH, MAX_REPLICATION_ENTRY_BATCH_BYTES,
    MAX_REPLICATION_SNAPSHOT_BYTES, ReplicationEntry, ReplicationEntryBatch, ReplicationError,
    ReplicationLog, ReplicationProgress, ReplicationSnapshot,
};
use std::io::{self, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::thread;
use std::time::Duration;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_RETRY_ATTEMPTS: usize = 8;
const MAX_RETRY_BACKOFF: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplicationRetryPolicy {
    max_attempts: usize,
    initial_backoff: Duration,
    max_backoff: Duration,
}

impl ReplicationRetryPolicy {
    pub fn new(
        max_attempts: usize,
        initial_backoff: Duration,
        max_backoff: Duration,
    ) -> Result<Self, ReplicationHttpError> {
        if !(1..=MAX_RETRY_ATTEMPTS).contains(&max_attempts) {
            return Err(ReplicationHttpError::InvalidConfig(format!(
                "retry attempts must be between 1 and {MAX_RETRY_ATTEMPTS}"
            )));
        }
        if initial_backoff > max_backoff || max_backoff > MAX_RETRY_BACKOFF {
            return Err(ReplicationHttpError::InvalidConfig(
                "retry backoff must be ordered and at most 30 seconds".into(),
            ));
        }
        Ok(Self {
            max_attempts,
            initial_backoff,
            max_backoff,
        })
    }
}

impl Default for ReplicationRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(50),
            max_backoff: Duration::from_secs(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReplicationHttpClient {
    host: String,
    port: u16,
    host_header: String,
    base_path: String,
    timeout: Duration,
    bearer_token: Option<String>,
    retry_policy: ReplicationRetryPolicy,
}

impl ReplicationHttpClient {
    /// Creates a client for the existing HTTP replication routes.
    ///
    /// Only plain http URLs are accepted. TLS, discovery, and durable retry
    /// queues belong to the future consensus transport rather than this
    /// bounded delivery client.
    pub fn new(base_url: &str) -> Result<Self, ReplicationHttpError> {
        let (host, port, host_header, base_path) = parse_base_url(base_url)?;
        Ok(Self {
            host,
            port,
            host_header,
            base_path,
            timeout: DEFAULT_TIMEOUT,
            bearer_token: None,
            retry_policy: ReplicationRetryPolicy::default(),
        })
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Result<Self, ReplicationHttpError> {
        if timeout.is_zero() {
            return Err(ReplicationHttpError::InvalidConfig(
                "timeout must be positive".into(),
            ));
        }
        self.timeout = timeout;
        Ok(self)
    }

    pub fn with_bearer_token(
        mut self,
        token: impl Into<String>,
    ) -> Result<Self, ReplicationHttpError> {
        let token = token.into();
        if !is_valid_bearer_token(&token) {
            return Err(ReplicationHttpError::InvalidConfig(
                "bearer token contains unsupported characters".into(),
            ));
        }
        self.bearer_token = Some(token);
        Ok(self)
    }

    pub fn with_retry_policy(mut self, retry_policy: ReplicationRetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub fn status(&self) -> Result<ReplicationHttpStatus, ReplicationHttpError> {
        let body = self.request(
            "GET",
            "/replication/status",
            None,
            MAX_REPLICATION_ENTRY_BATCH_BYTES,
        )?;
        let status: ReplicationHttpStatus = parse_json(&body, "status")?;
        status.validate()?;
        Ok(status)
    }

    pub fn entries(
        &self,
        after_index: u64,
        limit: usize,
    ) -> Result<ReplicationEntryBatch, ReplicationHttpError> {
        if !(1..=MAX_REPLICATION_ENTRY_BATCH).contains(&limit) {
            return Err(ReplicationError::Invalid(format!(
                "replication entry batch limit must be between 1 and {MAX_REPLICATION_ENTRY_BATCH}"
            ))
            .into());
        }
        let path = format!("/replication/entries?after={after_index}&limit={limit}");
        let body = self.request("GET", &path, None, MAX_REPLICATION_ENTRY_BATCH_BYTES)?;
        ReplicationEntryBatch::from_json(&body).map_err(Into::into)
    }

    pub fn snapshot(&self) -> Result<ReplicationSnapshot, ReplicationHttpError> {
        let body = self.request(
            "GET",
            "/replication/snapshot",
            None,
            MAX_REPLICATION_SNAPSHOT_BYTES,
        )?;
        ReplicationSnapshot::from_json(&body).map_err(Into::into)
    }

    pub fn deliver_entry(
        &self,
        entry: &ReplicationEntry,
    ) -> Result<ReplicationDeliveryResponse, ReplicationHttpError> {
        let body = entry.to_json()?;
        let response = self.request(
            "POST",
            "/replication/entry",
            Some(&body),
            MAX_REPLICATION_ENTRY_BATCH_BYTES,
        )?;
        let response: ReplicationDeliveryResponse = parse_json(&response, "entry delivery")?;
        response.validate()?;
        Ok(response)
    }

    pub fn install_snapshot(
        &self,
        snapshot: &ReplicationSnapshot,
    ) -> Result<ReplicationDeliveryResponse, ReplicationHttpError> {
        let body = snapshot.to_json()?;
        let response = self.request(
            "POST",
            "/replication/snapshot",
            Some(&body),
            MAX_REPLICATION_SNAPSHOT_BYTES,
        )?;
        let response: ReplicationDeliveryResponse = parse_json(&response, "snapshot delivery")?;
        response.validate()?;
        Ok(response)
    }

    pub fn acknowledge_progress(
        &self,
        progress: &ReplicationProgress,
    ) -> Result<ReplicationProgressResponse, ReplicationHttpError> {
        let body = progress.to_json()?;
        let response = self.request(
            "POST",
            "/replication/progress",
            Some(&body),
            MAX_REPLICATION_ENTRY_BATCH_BYTES,
        )?;
        let response: ReplicationProgressResponse = parse_json(&response, "progress")?;
        response.validate()?;
        Ok(response)
    }

    /// Pulls retained pages from an authority, installing a snapshot when the
    /// local cursor has fallen behind compaction, then acknowledges progress.
    pub fn catch_up(
        &self,
        catalog: &mut Catalog,
        log: &mut ReplicationLog,
        follower_id: String,
        limit: usize,
    ) -> Result<ReplicationSyncResult, ReplicationHttpError> {
        if !(1..=MAX_REPLICATION_ENTRY_BATCH).contains(&limit) {
            return Err(ReplicationError::Invalid(format!(
                "replication entry batch limit must be between 1 and {MAX_REPLICATION_ENTRY_BATCH}"
            ))
            .into());
        }

        let status = self.status()?;
        if status.role != "authority" {
            return Err(ReplicationError::Invalid(
                "replication catch-up requires an authority endpoint".into(),
            )
            .into());
        }
        if status.term != log.term() {
            return Err(ReplicationError::TermMismatch {
                expected: log.term(),
                actual: status.term,
            }
            .into());
        }
        let (_, local_schema_tag) = catalog
            .schema_representation()
            .map_err(ReplicationError::Catalog)?;
        if status.schema_tag != local_schema_tag {
            return Err(ReplicationError::SchemaMismatch {
                expected: local_schema_tag,
                actual: status.schema_tag,
            }
            .into());
        }
        if status.last_index < log.last_index() {
            return Err(ReplicationError::EntriesUnavailable {
                requested: log.last_index(),
                applied: status.last_index,
            }
            .into());
        }

        let target_index = status.last_index;
        let mut snapshot_installed = false;
        let mut snapshot_attempted = false;
        let mut entries_applied = 0;

        if log.last_index() < status.base_index {
            install_local_snapshot(self, catalog, log, &mut snapshot_installed)?;
            snapshot_attempted = true;
        }

        while log.last_index() < target_index {
            let requested_after = log.last_index();
            let batch = match self.entries(requested_after, limit) {
                Ok(batch) => batch,
                Err(error) if is_history_unavailable(&error) && !snapshot_attempted => {
                    install_local_snapshot(self, catalog, log, &mut snapshot_installed)?;
                    snapshot_attempted = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if batch.after_index != requested_after {
                return Err(ReplicationError::Invalid(format!(
                    "replication entry page starts at {}, expected {requested_after}",
                    batch.after_index
                ))
                .into());
            }
            if batch.entries.is_empty() {
                return Err(ReplicationError::Invalid(
                    "replication entry page made no progress".into(),
                )
                .into());
            }
            let before = log.last_index();
            let outcomes = log.receive_batch(catalog, batch.clone())?;
            entries_applied += outcomes
                .iter()
                .filter(|outcome| matches!(outcome, ApplyOutcome::Applied { .. }))
                .count();
            if log.last_index() <= before {
                return Err(ReplicationError::Invalid(
                    "replication entry page did not advance the local log".into(),
                )
                .into());
            }
            if batch.next_after.is_none() && log.last_index() < target_index {
                return Err(ReplicationError::Invalid(
                    "replication entry page ended before the authority position".into(),
                )
                .into());
            }
        }

        let progress = log.progress_for(catalog, follower_id)?;
        let progress_response = self.acknowledge_progress(&progress)?;
        Ok(ReplicationSyncResult {
            snapshot_installed,
            entries_applied,
            progress: progress_response,
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        max_response_body_bytes: usize,
    ) -> Result<Vec<u8>, ReplicationHttpError> {
        let mut attempt = 1;
        let mut backoff = self.retry_policy.initial_backoff;
        loop {
            match self.request_once(method, path, body, max_response_body_bytes) {
                Ok(response) => return Ok(response),
                Err(error) if attempt < self.retry_policy.max_attempts && error.is_retryable() => {
                    if !backoff.is_zero() {
                        thread::sleep(backoff);
                    }
                    attempt += 1;
                    backoff = backoff
                        .checked_mul(2)
                        .unwrap_or(self.retry_policy.max_backoff)
                        .min(self.retry_policy.max_backoff);
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn request_once(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        max_response_body_bytes: usize,
    ) -> Result<Vec<u8>, ReplicationHttpError> {
        let target = self.target(path)?;
        let mut stream = self.connect()?;
        stream.set_read_timeout(Some(self.timeout))?;
        stream.set_write_timeout(Some(self.timeout))?;

        let mut headers = format!(
            "{method} {target} HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.host_header
        );
        if let Some(token) = &self.bearer_token {
            headers.push_str("Authorization: Bearer ");
            headers.push_str(token);
            headers.push_str("\r\n");
        }
        if let Some(body) = body {
            headers.push_str("Content-Type: application/json\r\nContent-Length: ");
            headers.push_str(&body.len().to_string());
            headers.push_str("\r\n");
        }
        headers.push_str("\r\n");
        stream.write_all(headers.as_bytes())?;
        if let Some(body) = body {
            stream.write_all(body)?;
        }
        stream.flush()?;
        read_response(&mut stream, max_response_body_bytes)
    }

    fn connect(&self) -> Result<TcpStream, ReplicationHttpError> {
        let addresses: Vec<_> = (self.host.as_str(), self.port).to_socket_addrs()?.collect();
        if addresses.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "replication HTTP host resolved to no addresses",
            )
            .into());
        }
        let mut last_error = None;
        for address in addresses {
            match TcpStream::connect_timeout(&address, self.timeout) {
                Ok(stream) => return Ok(stream),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error
            .unwrap_or_else(|| {
                io::Error::new(io::ErrorKind::ConnectionRefused, "connection failed")
            })
            .into())
    }

    fn target(&self, path: &str) -> Result<String, ReplicationHttpError> {
        if !path.starts_with('/') || path.contains([' ', '\r', '\n']) {
            return Err(ReplicationHttpError::InvalidConfig(
                "replication endpoint path is invalid".into(),
            ));
        }
        if self.base_path == "/" {
            Ok(path.to_owned())
        } else {
            Ok(format!("{}{}", self.base_path, path))
        }
    }
}

impl ReplicationHttpError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::Io(_) => true,
            Self::HttpStatus { status, .. } => matches!(*status, 408 | 429 | 500 | 502 | 503 | 504),
            _ => false,
        }
    }
}

fn install_local_snapshot(
    client: &ReplicationHttpClient,
    catalog: &mut Catalog,
    log: &mut ReplicationLog,
    installed: &mut bool,
) -> Result<(), ReplicationHttpError> {
    let snapshot = client.snapshot()?;
    let outcome = log.install_snapshot(catalog, snapshot)?;
    *installed = matches!(
        outcome,
        ApplyOutcome::SnapshotInstalled { .. } | ApplyOutcome::SnapshotDuplicate { .. }
    );
    Ok(())
}

fn is_history_unavailable(error: &ReplicationHttpError) -> bool {
    matches!(
        error,
        ReplicationHttpError::HttpStatus {
            status: 409,
            code: Some(code),
            ..
        } if code == "replication_history_unavailable"
    )
}
