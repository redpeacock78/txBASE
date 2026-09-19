use super::{HttpResponse, error, header, json_response, query_result_response, read_json_body};
use crate::catalog::Catalog;
use crate::query::join::{self, JoinError};
use serde_json::Value;
use std::path::Path;
use tiny_http::{Method, Request, Server};

pub(super) fn serve(root: impl AsRef<Path>, bind: &str) -> Result<(), String> {
    let catalog =
        Catalog::from_path(root).map_err(|error| format!("cannot open catalog: {error}"))?;
    let server = Server::http(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
    eprintln!("listening on http://{bind}");
    for request in server.incoming_requests() {
        handle_request(request, &catalog);
    }
    Ok(())
}

fn handle_request(mut request: Request, catalog: &Catalog) {
    let path = request.url().split('?').next().unwrap_or("/").to_owned();
    let response = if matches!(request.method(), Method::Get) && path == "/catalog" {
        schema_response(catalog)
    } else if request.method().as_str() == "QUERY" && path == "/join" {
        join_response(&mut request, catalog)
    } else {
        json_response(
            405,
            error(
                "method_not_allowed",
                "only GET /catalog and QUERY /join are available",
            ),
            true,
        )
        .with_header(header("Allow", "GET, QUERY"))
    };
    if let Err(error) = request.respond(response) {
        eprintln!("failed to send HTTP response: {error}");
    }
}

pub(super) fn schema_response(catalog: &Catalog) -> HttpResponse {
    match catalog.schema_json() {
        Ok(schema) => json_response(200, schema, false),
        Err(catalog_error) => json_response(
            500,
            error("catalog_error", &catalog_error.to_string()),
            false,
        ),
    }
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
