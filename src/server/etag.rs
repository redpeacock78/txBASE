use super::{DbfTable, HttpResponse, Response, error, header, json_response, request_header};
use tiny_http::Request;

pub(super) fn current(table: &DbfTable) -> String {
    let mut representation = table.to_bytes();
    representation.extend_from_slice(
        &serde_json::to_vec(&table.active_json()).expect("DBF values must be JSON serializable"),
    );
    format!("\"txbase-{:016x}\"", fnv1a(&representation))
}

pub(super) fn with_current(response: HttpResponse, table: &DbfTable) -> HttpResponse {
    response.with_header(header("ETag", &current(table)))
}

pub(super) fn not_modified(
    request: &Request,
    table: &DbfTable,
    resource_exists: bool,
) -> Option<HttpResponse> {
    let value = request_header(request, "If-None-Match")?;
    let tag = current(table);
    if !matches_if_none_match(value, &tag, resource_exists) {
        return None;
    }
    Some(
        Response::from_data(Vec::<u8>::new())
            .with_status_code(304)
            .with_header(header("ETag", &tag)),
    )
}

pub(super) fn require_if_match(
    request: &Request,
    table: &DbfTable,
    resource_exists: bool,
) -> Result<(), HttpResponse> {
    let Some(value) = request_header(request, "If-Match") else {
        return Ok(());
    };
    let tag = current(table);
    if matches_if_match(value, &tag, resource_exists) {
        return Ok(());
    }
    Err(json_response(
        412,
        error(
            "precondition_failed",
            "If-Match does not match the current representation",
        ),
        false,
    )
    .with_header(header("ETag", &tag)))
}

fn matches_if_match(value: &str, current: &str, resource_exists: bool) -> bool {
    let tags = value.split(',').map(str::trim).collect::<Vec<_>>();
    if tags.len() == 1 && tags[0] == "*" {
        return resource_exists;
    }
    if tags.iter().any(|tag| *tag == "*" || tag.starts_with("W/")) {
        return false;
    }
    resource_exists && tags.contains(&current)
}

fn matches_if_none_match(value: &str, current: &str, resource_exists: bool) -> bool {
    let tags = value.split(',').map(str::trim).collect::<Vec<_>>();
    if tags.len() == 1 && tags[0] == "*" {
        return resource_exists;
    }
    resource_exists
        && tags
            .iter()
            .any(|tag| tag.strip_prefix("W/").unwrap_or(tag) == current)
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
