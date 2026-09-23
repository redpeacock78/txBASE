use super::{HttpResponse, error, json_response, read_json_body};
use crate::query;
use std::path::Path;
use tiny_http::Request;

pub(super) fn response(request: &mut Request, dbf_path: &Path) -> HttpResponse {
    response_with_path(request, Some(dbf_path))
}

pub(super) fn response_for_snapshot(request: &mut Request) -> HttpResponse {
    response_with_path(request, None)
}

fn response_with_path(request: &mut Request, dbf_path: Option<&Path>) -> HttpResponse {
    let body = match read_json_body(request, "QUERY /explain", true) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let query = match query::parse(&body) {
        Ok(query) => query,
        Err(query_error) => {
            return json_response(422, error("invalid_query", &query_error.to_string()), true);
        }
    };
    let explanation = match dbf_path {
        Some(dbf_path) => query::explain_query_details_at(dbf_path, &query),
        None => Ok(query::QueryExplanation {
            plan: query::QueryPlan::TableScan,
            cost: None,
        }),
    };
    match explanation {
        Ok(explanation) => json_response(200, serde_json::json!(explanation), true),
        Err(query_error) => {
            json_response(422, error("invalid_query", &query_error.to_string()), true)
        }
    }
}
