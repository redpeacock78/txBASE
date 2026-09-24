use super::CatalogReplicationRole;
use super::records::get_response;
#[cfg(test)]
use super::records::{
    delete_response_with_validator, post_response_at_with_validator, update_response_with_validator,
};
use super::{
    HttpResponse, error, etag, header, json_response, options_response, query_response_at,
    query_result_response, read_json_body,
};
use crate::catalog::{Catalog, CatalogError};
#[cfg(test)]
use crate::dbf::DbfTable;
use crate::query::join::{self, JoinError};
use serde_json::Value;
#[cfg(test)]
use std::collections::BTreeMap;
use std::path::Path;
use tiny_http::{Method, Request, Server};

pub(super) fn serve(
    root: impl AsRef<Path>,
    bind: &str,
    replication_term: u64,
    role: CatalogReplicationRole,
) -> Result<(), String> {
    let mut catalog =
        Catalog::from_path(root).map_err(|error| format!("cannot open catalog: {error}"))?;
    let mut replication = crate::replication::ReplicationLog::open(&catalog, replication_term)
        .map_err(|error| format!("cannot open replication log: {error}"))?;
    let server = Server::http(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
    eprintln!("listening on http://{bind}");
    for request in server.incoming_requests() {
        handle_request(request, &mut catalog, &mut replication, role);
    }
    Ok(())
}

fn handle_request(
    mut request: Request,
    catalog: &mut Catalog,
    replication: &mut crate::replication::ReplicationLog,
    role: CatalogReplicationRole,
) {
    let url = request.url().to_owned();
    let path = url.split('?').next().unwrap_or("/").to_owned();
    let response = if request.method().as_str() == "OPTIONS" {
        options_response("GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY")
    } else if let Some(response) =
        super::replication::response(&mut request, &path, catalog, replication, role)
    {
        response
    } else if role == CatalogReplicationRole::Follower
        && is_catalog_mutation(request.method(), &path)
    {
        super::replication::follower_read_only_response()
    } else if matches!(request.method(), Method::Get | Method::Head) && path == "/cdc" {
        super::cdc::catalog_response(&url, catalog)
    } else {
        match catalog_for_url(&url, request.method(), catalog) {
            Err(response) => response,
            Ok(snapshot) => {
                let catalog = snapshot.as_ref().unwrap_or(catalog);
                if matches!(request.method(), Method::Get | Method::Head) && path == "/catalog" {
                    schema_response(&request, catalog)
                } else if matches!(request.method(), Method::Get | Method::Head)
                    && record_route(&path).is_some()
                {
                    table_response(&request, &path, catalog)
                } else if request.method().as_str() == "QUERY" && path == "/join" {
                    join_response(&mut request, catalog)
                } else if request.method().as_str() == "QUERY"
                    && table_explain_route(&path).is_some()
                {
                    table_explain_response(&mut request, &path, catalog)
                } else if request.method().as_str() == "QUERY" && record_route(&path).is_some() {
                    table_query_response(&mut request, &path, catalog)
                } else if matches!(request.method(), Method::Post) && path == "/transaction" {
                    super::catalog_transaction::response_with_replication(
                        &mut request,
                        catalog,
                        Some(replication),
                    )
                } else if matches!(
                    request.method(),
                    Method::Post | Method::Put | Method::Patch | Method::Delete
                ) && record_route(&path).is_some()
                {
                    super::catalog_mutation::response(&mut request, &path, catalog, replication)
                } else {
                    json_response(
                        405,
                        error(
                            "method_not_allowed",
                            "only GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, and QUERY catalog routes are available",
                        ),
                        true,
                    )
                    .with_header(header(
                        "Allow",
                        "GET, HEAD, OPTIONS, POST, PUT, PATCH, DELETE, QUERY",
                    ))
                }
            }
        }
    };
    if let Err(error) = request.respond(response) {
        eprintln!("failed to send HTTP response: {error}");
    }
}

pub(super) fn table_response(request: &Request, path: &str, catalog: &Catalog) -> HttpResponse {
    let Some((name, local_path)) = record_route(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    if catalog.table_path(name).is_none() {
        return json_response(404, error("not_found", "table not found"), false);
    }
    let _lock = match catalog.acquire_read_lock() {
        Ok(lock) => lock,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    let table = match catalog.open_table_unlocked(name) {
        Ok(table) => table,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    get_response(request, &local_path, &table)
}

pub(super) fn table_query_response(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
) -> HttpResponse {
    let Some((name, local_path)) = record_route(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    if !matches!(local_path.as_str(), "/records" | "/records/stream") {
        return json_response(404, error("not_found", "resource not found"), false);
    }
    let Some(dbf_path) = catalog.table_path(name) else {
        return json_response(404, error("not_found", "table not found"), false);
    };
    let _lock = match catalog.acquire_read_lock() {
        Ok(lock) => lock,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    let table = match catalog.open_table_unlocked(name) {
        Ok(table) => table,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    if local_path == "/records/stream" {
        super::stream::response(request, &table)
    } else if catalog.is_historical() {
        super::query_response_without_path(request, &local_path, &table)
    } else {
        query_response_at(request, &local_path, &table, dbf_path)
    }
}

#[cfg(test)]
pub(super) fn table_mutation_response(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
) -> HttpResponse {
    if catalog.is_historical() {
        return json_response(
            405,
            error(
                "historical_snapshot_read_only",
                "historical catalog snapshots are read-only",
            ),
            false,
        )
        .with_header(header("Allow", "GET, HEAD, QUERY"));
    }
    let Some((name, local_path)) = record_route(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    let Some(dbf_path) = catalog.table_path(name) else {
        return json_response(404, error("not_found", "table not found"), false);
    };
    let _lock = match catalog.acquire_write_lock() {
        Ok(lock) => lock,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    let mut table = match catalog.open_table_unlocked(name) {
        Ok(table) => table,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    let validate = |candidate: &DbfTable| {
        let mut replacements = BTreeMap::new();
        replacements.insert(name.to_owned(), candidate.clone());
        catalog
            .validate_replacements(&replacements)
            .map_err(|error| error.to_string())
    };
    match request.method() {
        Method::Post => post_response_at_with_validator(
            request,
            &local_path,
            path,
            &mut table,
            dbf_path,
            Some(&validate),
        ),
        Method::Put => update_response_with_validator(
            request,
            &local_path,
            &mut table,
            dbf_path,
            true,
            Some(&validate),
        ),
        Method::Patch => update_response_with_validator(
            request,
            &local_path,
            &mut table,
            dbf_path,
            false,
            Some(&validate),
        ),
        Method::Delete => delete_response_with_validator(
            request,
            &local_path,
            &mut table,
            dbf_path,
            Some(&validate),
        ),
        _ => json_response(
            405,
            error(
                "method_not_allowed",
                "only POST, PUT, PATCH, and DELETE table routes are available",
            ),
            true,
        )
        .with_header(header("Allow", "POST, PUT, PATCH, DELETE")),
    }
}

pub(super) fn table_explain_response(
    request: &mut Request,
    path: &str,
    catalog: &Catalog,
) -> HttpResponse {
    let Some(name) = table_explain_route(path) else {
        return json_response(404, error("not_found", "resource not found"), false);
    };
    let Some(dbf_path) = catalog.table_path(name) else {
        return json_response(404, error("not_found", "table not found"), false);
    };
    let _lock = match catalog.acquire_read_lock() {
        Ok(lock) => lock,
        Err(catalog_error) => {
            return json_response(
                500,
                error("catalog_error", &catalog_error.to_string()),
                false,
            );
        }
    };
    if let Err(catalog_error) = catalog.open_table_unlocked(name) {
        return json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        );
    }
    if catalog.is_historical() {
        super::explain::response_for_snapshot(request)
    } else {
        super::explain::response(request, dbf_path)
    }
}

pub(super) fn catalog_for_url(
    url: &str,
    method: &Method,
    catalog: &Catalog,
) -> Result<Option<Catalog>, HttpResponse> {
    let Some(transaction_id) = snapshot_parameter(url)? else {
        return Ok(None);
    };
    if !matches!(method, Method::Get | Method::Head) && method.as_str() != "QUERY" {
        return Err(json_response(
            405,
            error(
                "historical_snapshot_read_only",
                "the at parameter is available only on catalog reads",
            ),
            false,
        )
        .with_header(header("Allow", "GET, HEAD, QUERY")));
    }
    Catalog::from_path_at(catalog.root(), transaction_id)
        .map(Some)
        .map_err(snapshot_error_response)
}

fn snapshot_parameter(url: &str) -> Result<Option<u64>, HttpResponse> {
    let query = url.split_once('?').map_or("", |(_, query)| query);
    let mut transaction_id = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let Some((name, value)) = pair.split_once('=') else {
            if pair == "at" {
                return Err(invalid_snapshot_parameter("at must be a positive integer"));
            }
            continue;
        };
        if name != "at" {
            continue;
        }
        if transaction_id.is_some() {
            return Err(invalid_snapshot_parameter("the at parameter was repeated"));
        }
        let parsed = value
            .parse::<u64>()
            .map_err(|_| invalid_snapshot_parameter("at must be a positive integer"))?;
        if parsed == 0 {
            return Err(invalid_snapshot_parameter("at must be a positive integer"));
        }
        transaction_id = Some(parsed);
    }
    Ok(transaction_id)
}

fn invalid_snapshot_parameter(message: &str) -> HttpResponse {
    json_response(400, error("invalid_snapshot", message), false)
}

fn snapshot_error_response(catalog_error: CatalogError) -> HttpResponse {
    match catalog_error {
        CatalogError::Invalid(message) => {
            json_response(422, error("invalid_snapshot", &message), false)
        }
        catalog_error => json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        ),
    }
}

pub(super) fn record_route(path: &str) -> Option<(&str, String)> {
    let segments = path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        [table, "records"] if !table.is_empty() => Some((table, String::from("/records"))),
        [table, "records", "stream"] if !table.is_empty() => {
            Some((table, String::from("/records/stream")))
        }
        [table, "records", record] if !table.is_empty() && !record.is_empty() => {
            Some((table, format!("/records/{record}")))
        }
        _ => None,
    }
}

fn table_explain_route(path: &str) -> Option<&str> {
    let segments = path.strip_prefix('/')?.split('/').collect::<Vec<_>>();
    match segments.as_slice() {
        [table, "explain"] if !table.is_empty() => Some(table),
        _ => None,
    }
}

pub(super) fn schema_response(request: &Request, catalog: &Catalog) -> HttpResponse {
    match catalog.schema_representation() {
        Ok((schema, tag)) => {
            if let Some(response) = etag::not_modified_for_tag(request, &tag, true) {
                return response;
            }
            etag::with_tag(json_response(200, schema, false), &tag)
        }
        Err(catalog_error) => json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        ),
    }
}

fn is_catalog_mutation(method: &Method, path: &str) -> bool {
    matches!(
        method,
        Method::Post | Method::Put | Method::Patch | Method::Delete
    ) && (path == "/transaction" || record_route(path).is_some())
}

pub(super) fn join_response(request: &mut Request, catalog: &Catalog) -> HttpResponse {
    let body = match read_json_body(request, "QUERY /join", true) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let join = match join::parse(&body) {
        Ok(join) => join,
        Err(error) => return join_error_response(error),
    };
    match join::execute(catalog, &join) {
        Ok(records) => query_result_response(request, Value::Array(records)),
        Err(error) => join_error_response(error),
    }
}

fn join_error_response(join_error: JoinError) -> HttpResponse {
    let message = join_error.to_string();
    let (status, code) = match join_error {
        JoinError::Catalog(_) => (500, "catalog_error"),
        JoinError::InvalidJson(_) | JoinError::Query(_) | JoinError::Invalid(_) => {
            (422, "invalid_join")
        }
    };
    json_response(status, error(code, &message), true)
}
