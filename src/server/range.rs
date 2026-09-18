use super::{HttpResponse, header, json_bytes_response, json_response, request_header};
use serde_json::Value;
use tiny_http::Request;

enum ByteRange {
    Ignore,
    Partial { start: usize, end: usize },
    Unsatisfiable,
}

pub(super) fn query_result_response(request: &Request, value: Value) -> HttpResponse {
    let full_body = value.to_string().into_bytes();
    let range = request_header(request, "Range")
        .map(|value| parse_single_byte_range(value, full_body.len()))
        .unwrap_or(ByteRange::Ignore);
    match range {
        ByteRange::Partial { start, end } => {
            json_bytes_response(206, full_body[start..=end].to_vec(), true)
                .with_header(header("Accept-Ranges", "bytes"))
                .with_header(header(
                    "Content-Range",
                    &format!("bytes {start}-{end}/{}", full_body.len()),
                ))
        }
        ByteRange::Unsatisfiable => json_response(
            416,
            super::error(
                "range_not_satisfiable",
                "the requested byte range is not satisfiable",
            ),
            true,
        )
        .with_header(header("Accept-Ranges", "bytes"))
        .with_header(header(
            "Content-Range",
            &format!("bytes */{}", full_body.len()),
        )),
        ByteRange::Ignore => {
            json_bytes_response(200, full_body, true).with_header(header("Accept-Ranges", "bytes"))
        }
    }
}

fn parse_single_byte_range(value: &str, length: usize) -> ByteRange {
    if length == 0 {
        return ByteRange::Ignore;
    }
    let Some(specifier) = value.strip_prefix("bytes=") else {
        return ByteRange::Ignore;
    };
    if specifier.contains(',') {
        return ByteRange::Ignore;
    }
    let Some((start, end)) = specifier.split_once('-') else {
        return ByteRange::Ignore;
    };
    if start.is_empty() {
        let Some(suffix) = end.parse::<u128>().ok() else {
            return ByteRange::Ignore;
        };
        if suffix == 0 {
            return ByteRange::Unsatisfiable;
        }
        let suffix = usize::try_from(suffix).unwrap_or(length).min(length);
        return ByteRange::Partial {
            start: length - suffix,
            end: length - 1,
        };
    }

    let Some(start) = start.parse::<u128>().ok() else {
        return ByteRange::Ignore;
    };
    if start >= length as u128 {
        return ByteRange::Unsatisfiable;
    }
    let start = start as usize;
    if end.is_empty() {
        return ByteRange::Partial {
            start,
            end: length - 1,
        };
    }
    let Some(end) = end.parse::<u128>().ok() else {
        return ByteRange::Ignore;
    };
    if end < start as u128 {
        return ByteRange::Ignore;
    }
    let end = usize::try_from(end).unwrap_or(usize::MAX).min(length - 1);
    ByteRange::Partial { start, end }
}
