use super::{
    HttpResponse, JSON_MERGE_PATCH_MEDIA_TYPE, JSON_PATCH_MEDIA_TYPE, MAX_BODY, PatchMediaType,
    error, json_response,
};
use serde_json::{Map, Value};
use std::io::Read;
use tiny_http::Request;

pub fn read_json_object(
    request: &mut Request,
    operation: &str,
    accept_query: bool,
) -> Result<Map<String, Value>, HttpResponse> {
    let body = read_json_body(request, operation, accept_query)?;
    parse_json_object(&body, accept_query)
}

pub fn read_json_patch_document(
    request: &mut Request,
    operation: &str,
    accept_query: bool,
) -> Result<(Value, PatchMediaType), HttpResponse> {
    let (body, media_type) = read_json_body_with_options(request, operation, accept_query, true)?;
    let value = parse_json_value(&body, accept_query)?;
    let valid_root = match media_type {
        PatchMediaType::Json | PatchMediaType::MergePatch => value.is_object(),
        PatchMediaType::JsonPatch => value.is_array(),
    };
    if !valid_root {
        let message = match media_type {
            PatchMediaType::Json | PatchMediaType::MergePatch => "JSON body must be an object",
            PatchMediaType::JsonPatch => "JSON Patch body must be an array",
        };
        return Err(json_response(
            422,
            error("invalid_json", message),
            accept_query,
        ));
    }
    Ok((value, media_type))
}

fn parse_json_object(body: &[u8], accept_query: bool) -> Result<Map<String, Value>, HttpResponse> {
    let value = parse_json_value(body, accept_query)?;
    value.as_object().cloned().ok_or_else(|| {
        json_response(
            422,
            error("invalid_json", "JSON body must be an object"),
            accept_query,
        )
    })
}

fn parse_json_value(body: &[u8], accept_query: bool) -> Result<Value, HttpResponse> {
    serde_json::from_slice::<Value>(body).map_err(|parse_error| {
        json_response(
            422,
            error("invalid_json", &format!("invalid JSON body: {parse_error}")),
            accept_query,
        )
    })
}

pub fn read_json_body(
    request: &mut Request,
    operation: &str,
    accept_query: bool,
) -> Result<Vec<u8>, HttpResponse> {
    read_json_body_with_options(request, operation, accept_query, false).map(|(body, _)| body)
}

fn read_json_body_with_options(
    request: &mut Request,
    operation: &str,
    accept_query: bool,
    allow_patch_formats: bool,
) -> Result<(Vec<u8>, PatchMediaType), HttpResponse> {
    let Some(content_type) = content_type(request) else {
        let required = if allow_patch_formats {
            "application/json, application/merge-patch+json, or application/json-patch+json"
        } else {
            "application/json"
        };
        return Err(json_response(
            400,
            error(
                "missing_content_type",
                &format!("{operation} requires Content-Type: {required}"),
            ),
            accept_query,
        ));
    };
    let media_type = content_type.split(';').next().map(str::trim);
    let is_json = media_type
        .is_some_and(|media_type| media_type.eq_ignore_ascii_case(super::JSON_QUERY_MEDIA_TYPE));
    let is_merge_patch = allow_patch_formats
        && media_type
            .is_some_and(|media_type| media_type.eq_ignore_ascii_case(JSON_MERGE_PATCH_MEDIA_TYPE));
    let is_json_patch = allow_patch_formats
        && media_type
            .is_some_and(|media_type| media_type.eq_ignore_ascii_case(JSON_PATCH_MEDIA_TYPE));
    if !is_json && !is_merge_patch && !is_json_patch {
        let supported = if allow_patch_formats {
            "application/json, application/merge-patch+json, or application/json-patch+json"
        } else {
            "application/json"
        };
        return Err(json_response(
            415,
            error(
                "unsupported_media_type",
                &format!("only {supported} request content is supported"),
            ),
            accept_query,
        ));
    }
    let mut body = Vec::new();
    if request
        .as_reader()
        .take((MAX_BODY + 1) as u64)
        .read_to_end(&mut body)
        .is_err()
    {
        return Err(json_response(
            400,
            error("invalid_body", "could not read request content"),
            accept_query,
        ));
    }
    if body.len() > MAX_BODY {
        return Err(json_response(
            413,
            error("body_too_large", "request body exceeds 1 MiB"),
            accept_query,
        ));
    }
    let media_type = if is_merge_patch {
        PatchMediaType::MergePatch
    } else if is_json_patch {
        PatchMediaType::JsonPatch
    } else {
        PatchMediaType::Json
    };
    Ok((body, media_type))
}

pub fn request_header<'a>(request: &'a Request, name: &'static str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|header| header.field.equiv(name))
        .map(|header| header.value.as_str())
}

fn content_type(request: &Request) -> Option<&str> {
    request_header(request, "Content-Type")
}
