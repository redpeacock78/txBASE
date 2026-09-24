use crate::replication::{
    MAX_REPLICATION_FOLLOWERS, REPLICATION_TRANSPORT_VERSION, ReplicationError,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::io::{self, Read};
use std::net::TcpStream;

const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum ReplicationHttpError {
    InvalidUrl(String),
    UnsupportedScheme(String),
    InvalidConfig(String),
    Io(io::Error),
    InvalidResponse(String),
    HttpStatus {
        status: u16,
        code: Option<String>,
        message: String,
    },
    Replication(ReplicationError),
}

impl Display for ReplicationHttpError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(message) => write!(formatter, "invalid replication URL: {message}"),
            Self::UnsupportedScheme(scheme) => {
                write!(formatter, "unsupported replication URL scheme: {scheme}")
            }
            Self::InvalidConfig(message) => {
                write!(
                    formatter,
                    "invalid replication HTTP client configuration: {message}"
                )
            }
            Self::Io(error) => write!(formatter, "replication HTTP I/O error: {error}"),
            Self::InvalidResponse(message) => {
                write!(formatter, "invalid replication HTTP response: {message}")
            }
            Self::HttpStatus {
                status,
                code,
                message,
            } => match code {
                Some(code) => write!(formatter, "replication HTTP {status} {code}: {message}"),
                None => write!(formatter, "replication HTTP {status}: {message}"),
            },
            Self::Replication(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for ReplicationHttpError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Replication(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ReplicationHttpError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<ReplicationError> for ReplicationHttpError {
    fn from(error: ReplicationError) -> Self {
        Self::Replication(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationHttpStatus {
    pub transport_version: u16,
    pub role: String,
    pub term: u64,
    pub base_index: u64,
    pub base_transaction_id: u64,
    pub last_index: u64,
    pub last_transaction_id: u64,
    pub follower_count: usize,
    pub safe_compaction_index: Option<u64>,
    pub schema_tag: String,
}

impl ReplicationHttpStatus {
    pub fn validate(&self) -> Result<(), ReplicationHttpError> {
        if self.transport_version != REPLICATION_TRANSPORT_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication transport version: {}",
                self.transport_version
            ))
            .into());
        }
        if !matches!(self.role.as_str(), "authority" | "follower") {
            return Err(ReplicationError::Invalid("replication role is invalid".into()).into());
        }
        if self.term == 0
            || self.base_index > self.last_index
            || self.base_transaction_id > self.last_transaction_id
        {
            return Err(
                ReplicationError::Invalid("replication status position is invalid".into()).into(),
            );
        }
        let expected_last_transaction_id = self
            .base_transaction_id
            .checked_add(self.last_index - self.base_index)
            .ok_or_else(|| {
                ReplicationError::Invalid("replication transaction ID exhausted".into())
            })?;
        if self.last_transaction_id != expected_last_transaction_id {
            return Err(ReplicationError::TransactionGap {
                expected: expected_last_transaction_id,
                actual: self.last_transaction_id,
            }
            .into());
        }
        if self.follower_count > MAX_REPLICATION_FOLLOWERS {
            return Err(ReplicationError::Invalid(
                "replication follower count exceeds the configured maximum".into(),
            )
            .into());
        }
        if self
            .safe_compaction_index
            .is_some_and(|index| index > self.last_index)
        {
            return Err(ReplicationError::Invalid(
                "safe compaction index exceeds the last replication index".into(),
            )
            .into());
        }
        if self.schema_tag.trim().is_empty() {
            return Err(ReplicationError::Invalid("schema_tag must not be empty".into()).into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationDeliveryOutcome {
    Applied,
    Duplicate,
    SnapshotInstalled,
    SnapshotDuplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationDeliveryResponse {
    pub transport_version: u16,
    pub outcome: ReplicationDeliveryOutcome,
    pub index: u64,
    pub transaction_id: u64,
}

impl ReplicationDeliveryResponse {
    pub fn validate(&self) -> Result<(), ReplicationHttpError> {
        if self.transport_version != REPLICATION_TRANSPORT_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication transport version: {}",
                self.transport_version
            ))
            .into());
        }
        if self.index == 0 || self.transaction_id == 0 {
            return Err(ReplicationError::Invalid(
                "replication delivery position must be positive".into(),
            )
            .into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplicationProgressResponseOutcome {
    Accepted,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplicationProgressResponse {
    pub transport_version: u16,
    pub outcome: ReplicationProgressResponseOutcome,
    pub follower_id: String,
    pub index: u64,
    pub transaction_id: u64,
    pub safe_compaction_index: Option<u64>,
}

impl ReplicationProgressResponse {
    pub fn validate(&self) -> Result<(), ReplicationHttpError> {
        if self.transport_version != REPLICATION_TRANSPORT_VERSION {
            return Err(ReplicationError::Invalid(format!(
                "unsupported replication transport version: {}",
                self.transport_version
            ))
            .into());
        }
        if self.follower_id.is_empty()
            || self.follower_id.len() > 128
            || !self
                .follower_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(
                ReplicationError::Invalid("replication follower_id is invalid".into()).into(),
            );
        }
        if (self.index == 0) != (self.transaction_id == 0) {
            return Err(ReplicationError::Invalid(
                "progress index and transaction_id must both be zero or both be positive".into(),
            )
            .into());
        }
        if self
            .safe_compaction_index
            .is_some_and(|index| index > self.index)
        {
            return Err(ReplicationError::Invalid(
                "safe compaction index exceeds acknowledged progress".into(),
            )
            .into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicationSyncResult {
    pub snapshot_installed: bool,
    pub entries_applied: usize,
    pub progress: ReplicationProgressResponse,
}

pub(super) fn parse_base_url(
    base_url: &str,
) -> Result<(String, u16, String, String), ReplicationHttpError> {
    if base_url.is_empty()
        || base_url
            .chars()
            .any(|character| character.is_ascii_control() || character.is_ascii_whitespace())
    {
        return Err(ReplicationHttpError::InvalidUrl(
            "URL must not be empty or contain whitespace/control characters".into(),
        ));
    }
    let Some((scheme, rest)) = base_url.split_once("://") else {
        return Err(ReplicationHttpError::InvalidUrl(
            "URL must use the http:// scheme".into(),
        ));
    };
    if !scheme.eq_ignore_ascii_case("http") {
        return Err(ReplicationHttpError::UnsupportedScheme(scheme.into()));
    }
    let authority_end = rest.find(['/', '?', '#']);
    let (authority, suffix) = authority_end.map_or((rest, ""), |index| rest.split_at(index));
    if authority.is_empty() || authority.contains('@') {
        return Err(ReplicationHttpError::InvalidUrl(
            "URL authority must contain a host without user information".into(),
        ));
    }
    if suffix.contains('?') || suffix.contains('#') {
        return Err(ReplicationHttpError::InvalidUrl(
            "base URL must not contain a query or fragment".into(),
        ));
    }
    let path = if suffix.is_empty() { "/" } else { suffix };
    if !path.starts_with('/') || !path.is_ascii() {
        return Err(ReplicationHttpError::InvalidUrl(
            "base URL path must be an ASCII path".into(),
        ));
    }
    let base_path = path.trim_end_matches('/');
    let base_path = if base_path.is_empty() { "/" } else { base_path }.to_owned();

    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some(close) = rest.find(']') else {
            return Err(ReplicationHttpError::InvalidUrl(
                "bracketed IPv6 host is missing ]".into(),
            ));
        };
        let host = &rest[..close];
        let port = parse_port(&rest[close + 1..])?;
        if host.is_empty() {
            return Err(ReplicationHttpError::InvalidUrl(
                "host must not be empty".into(),
            ));
        }
        (host.to_owned(), port)
    } else {
        if authority.matches(':').count() > 1 {
            return Err(ReplicationHttpError::InvalidUrl(
                "IPv6 hosts must be enclosed in brackets".into(),
            ));
        }
        let (host, port) = if let Some((host, port)) = authority.split_once(':') {
            (host, parse_port_value(port)?)
        } else {
            (authority, 80)
        };
        if host.is_empty() {
            return Err(ReplicationHttpError::InvalidUrl(
                "host must not be empty".into(),
            ));
        }
        (host.to_owned(), port)
    };
    if host.bytes().any(|byte| {
        byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'[' || byte == b']'
    }) {
        return Err(ReplicationHttpError::InvalidUrl(
            "host contains an unsupported character".into(),
        ));
    }
    let host_header = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    Ok((host, port, host_header, base_path))
}

fn parse_port(rest: &str) -> Result<u16, ReplicationHttpError> {
    if rest.is_empty() {
        return Ok(80);
    }
    let Some(port) = rest.strip_prefix(':') else {
        return Err(ReplicationHttpError::InvalidUrl(
            "characters after a bracketed host must be a port".into(),
        ));
    };
    parse_port_value(port)
}

fn parse_port_value(port: &str) -> Result<u16, ReplicationHttpError> {
    let port = port.parse::<u16>().map_err(|_| {
        ReplicationHttpError::InvalidUrl("port must be an integer between 1 and 65535".into())
    })?;
    if port == 0 {
        return Err(ReplicationHttpError::InvalidUrl(
            "port must be an integer between 1 and 65535".into(),
        ));
    }
    Ok(port)
}

pub(super) fn is_valid_bearer_token(token: &str) -> bool {
    let mut has_token_byte = false;
    let mut padding = false;
    for byte in token.bytes() {
        match byte {
            b'=' => padding = true,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'+' | b'/'
                if !padding =>
            {
                has_token_byte = true
            }
            _ => return false,
        }
    }
    has_token_byte
}

pub(super) fn parse_json<T: DeserializeOwned>(
    body: &[u8],
    label: &str,
) -> Result<T, ReplicationHttpError> {
    serde_json::from_slice(body)
        .map_err(|error| ReplicationHttpError::InvalidResponse(format!("{label} JSON: {error}")))
}

pub(super) fn read_response(
    stream: &mut TcpStream,
    max_body_bytes: usize,
) -> Result<Vec<u8>, ReplicationHttpError> {
    let max_response_bytes = max_body_bytes.saturating_add(MAX_HTTP_HEADER_BYTES);
    let mut response = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let read = stream.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        if response.len().saturating_add(read) > max_response_bytes {
            return Err(ReplicationHttpError::InvalidResponse(
                "response exceeds the configured size limit".into(),
            ));
        }
        response.extend_from_slice(&buffer[..read]);
    }

    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .ok_or_else(|| {
            ReplicationHttpError::InvalidResponse("response headers are incomplete".into())
        })?;
    if header_end > MAX_HTTP_HEADER_BYTES {
        return Err(ReplicationHttpError::InvalidResponse(
            "response headers exceed the configured size limit".into(),
        ));
    }
    let header_text = std::str::from_utf8(&response[..header_end - 2]).map_err(|_| {
        ReplicationHttpError::InvalidResponse("response headers are not UTF-8".into())
    })?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines.next().ok_or_else(|| {
        ReplicationHttpError::InvalidResponse("response status line is missing".into())
    })?;
    let mut status_parts = status_line.splitn(3, ' ');
    let version = status_parts.next().unwrap_or_default();
    let status = status_parts
        .next()
        .ok_or_else(|| ReplicationHttpError::InvalidResponse("response status is missing".into()))?
        .parse::<u16>()
        .map_err(|_| ReplicationHttpError::InvalidResponse("response status is invalid".into()))?;
    if !version.starts_with("HTTP/") {
        return Err(ReplicationHttpError::InvalidResponse(
            "response HTTP version is invalid".into(),
        ));
    }

    let mut content_length = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReplicationHttpError::InvalidResponse(
                "response header is malformed".into(),
            ));
        };
        if name.eq_ignore_ascii_case("transfer-encoding") && !value.trim().is_empty() {
            return Err(ReplicationHttpError::InvalidResponse(
                "chunked transfer encoding is not supported".into(),
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let value = value.trim().parse::<usize>().map_err(|_| {
                ReplicationHttpError::InvalidResponse("content length is invalid".into())
            })?;
            if content_length.replace(value).is_some() {
                return Err(ReplicationHttpError::InvalidResponse(
                    "content length was repeated".into(),
                ));
            }
        }
    }
    let content_length = content_length
        .ok_or_else(|| ReplicationHttpError::InvalidResponse("content length is missing".into()))?;
    if content_length > max_body_bytes {
        return Err(ReplicationHttpError::InvalidResponse(
            "response body exceeds the configured size limit".into(),
        ));
    }
    let body = &response[header_end..];
    if body.len() != content_length {
        return Err(ReplicationHttpError::InvalidResponse(format!(
            "content length is {content_length}, received {} bytes",
            body.len()
        )));
    }
    if !(200..300).contains(&status) {
        return Err(http_status_error(status, body));
    }
    Ok(body.to_vec())
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Deserialize)]
struct ErrorBody {
    code: Option<String>,
    message: Option<String>,
}

fn http_status_error(status: u16, body: &[u8]) -> ReplicationHttpError {
    match serde_json::from_slice::<ErrorEnvelope>(body) {
        Ok(envelope) => ReplicationHttpError::HttpStatus {
            status,
            code: envelope.error.code,
            message: envelope
                .error
                .message
                .unwrap_or_else(|| "replication request failed".into()),
        },
        Err(_) => ReplicationHttpError::HttpStatus {
            status,
            code: None,
            message: "replication request failed".into(),
        },
    }
}
