use super::{HttpResponse, dbf_error_response, error, json_response};
use crate::catalog::{Catalog, CatalogError};
use crate::dbf::{DbfTable, MAX_CDC_PAGE_SIZE};
use serde::Serialize;
use serde_json::json;
use std::path::Path;

const DEFAULT_PAGE_SIZE: usize = 100;

#[derive(Clone, Copy)]
struct Parameters {
    after: Option<u64>,
    limit: usize,
}

pub(super) fn table_response(url: &str, dbf_path: &Path) -> HttpResponse {
    let parameters = match parse_parameters(url) {
        Ok(parameters) => parameters,
        Err(response) => return response,
    };
    match DbfTable::cdc_events_page(dbf_path, parameters.after, parameters.limit) {
        Ok((events, next_after)) => page_response(events, next_after),
        Err(error) => dbf_error_response(error),
    }
}

pub(super) fn catalog_response(url: &str, catalog: &Catalog) -> HttpResponse {
    let parameters = match parse_parameters(url) {
        Ok(parameters) => parameters,
        Err(response) => return response,
    };
    match Catalog::cdc_events_page(catalog.root(), parameters.after, parameters.limit) {
        Ok((events, next_after)) => page_response(events, next_after),
        Err(error) => catalog_error_response(error),
    }
}

fn parse_parameters(url: &str) -> Result<Parameters, HttpResponse> {
    let query = url.split_once('?').map_or("", |(_, query)| query);
    let mut after = None;
    let mut limit = None;
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let Some((name, value)) = pair.split_once('=') else {
            return Err(invalid_parameters(
                "CDC query parameters must use name=value",
            ));
        };
        match name {
            "after" => {
                if after.is_some() {
                    return Err(invalid_parameters("CDC after parameter was repeated"));
                }
                after = Some(parse_positive(value, "after")?);
            }
            "limit" => {
                if limit.is_some() {
                    return Err(invalid_parameters("CDC limit parameter was repeated"));
                }
                let parsed = value
                    .parse::<usize>()
                    .map_err(|_| invalid_parameters("CDC limit must be a positive integer"))?;
                if !(1..=MAX_CDC_PAGE_SIZE).contains(&parsed) {
                    return Err(invalid_parameters(&format!(
                        "CDC limit must be between 1 and {MAX_CDC_PAGE_SIZE}"
                    )));
                }
                limit = Some(parsed);
            }
            _ => return Err(invalid_parameters("unknown CDC query parameter")),
        }
    }
    Ok(Parameters {
        after,
        limit: limit.unwrap_or(DEFAULT_PAGE_SIZE),
    })
}

fn parse_positive(value: &str, name: &str) -> Result<u64, HttpResponse> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| invalid_parameters(&format!("CDC {name} must be a positive integer")))?;
    if parsed == 0 {
        return Err(invalid_parameters(&format!(
            "CDC {name} must be a positive integer"
        )));
    }
    Ok(parsed)
}

fn invalid_parameters(message: &str) -> HttpResponse {
    json_response(400, error("invalid_cdc_cursor", message), false)
}

fn page_response<T: Serialize>(events: Vec<T>, next_after: Option<u64>) -> HttpResponse {
    match serde_json::to_value(events) {
        Ok(events) => json_response(
            200,
            json!({"events": events, "next_after": next_after}),
            false,
        ),
        Err(error) => json_response(
            500,
            super::error("serialization_error", &error.to_string()),
            false,
        ),
    }
}

fn catalog_error_response(error: CatalogError) -> HttpResponse {
    match error {
        CatalogError::Invalid(message) => {
            json_response(422, super::error("invalid_cdc", &message), false)
        }
        CatalogError::Io(error) => json_response(
            500,
            super::error("storage_error", &error.to_string()),
            false,
        ),
        CatalogError::Table { name, source } => json_response(
            500,
            super::error("storage_error", &format!("table {name}: {source}")),
            false,
        ),
    }
}
