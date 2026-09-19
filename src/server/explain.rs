use super::{HttpResponse, error, json_response, read_json_body};
use crate::query;
use std::path::Path;
use tiny_http::Request;

pub(super) fn response(request: &mut Request, dbf_path: &Path) -> HttpResponse {
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
    match query::explain_query_at(dbf_path, &query) {
        Ok(plan) => json_response(200, serde_json::json!({"plan": plan}), true),
        Err(query_error) => {
            json_response(422, error("invalid_query", &query_error.to_string()), true)
        }
    }
}
