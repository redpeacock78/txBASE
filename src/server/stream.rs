use super::{HttpResponse, ServerBody, error, header, json_response, read_json_body};
use crate::dbf::DbfTable;
use crate::query;
use serde_json::Value;
use std::fmt::Display;
use std::io::{self, Read};
use tiny_http::{Request, Response, StatusCode};

const NDJSON_MEDIA_TYPE: &str = "application/x-ndjson";
pub(super) const CHANNEL_CAPACITY: usize = 32;

pub(super) fn response(request: &mut Request, table: &DbfTable) -> HttpResponse {
    let body = match read_json_body(request, "QUERY /records/stream", true) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let query = match query::parse(&body) {
        Ok(query) => query,
        Err(query_error) => {
            return json_response(422, error("invalid_query", &query_error.to_string()), true);
        }
    };
    let stream = match query::stream_query_bounded(table, &query, CHANNEL_CAPACITY) {
        Ok(stream) => stream,
        Err(query_error) => {
            return json_response(422, error("invalid_query", &query_error.to_string()), true);
        }
    };
    ndjson_response(stream)
}

pub(super) fn ndjson_response<E>(
    stream: impl Iterator<Item = Result<Value, E>> + Send + 'static,
) -> HttpResponse
where
    E: Display + Send + 'static,
{
    Response::new(
        StatusCode(200),
        vec![
            header("Content-Type", NDJSON_MEDIA_TYPE),
            header("Accept-Query", "\"application/json\""),
        ],
        ServerBody::Stream(Box::new(NdjsonReader::new(stream))),
        None,
        None,
    )
}

struct NdjsonReader<E> {
    stream: Box<dyn Iterator<Item = Result<Value, E>> + Send>,
    pending: Vec<u8>,
    offset: usize,
    done: bool,
}

impl<E: 'static> NdjsonReader<E> {
    fn new(stream: impl Iterator<Item = Result<Value, E>> + Send + 'static) -> Self {
        Self {
            stream: Box::new(stream),
            pending: Vec::new(),
            offset: 0,
            done: false,
        }
    }
}

impl<E: Display> Read for NdjsonReader<E> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        loop {
            if self.offset < self.pending.len() {
                let count = (self.pending.len() - self.offset).min(buffer.len());
                buffer[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
                self.offset += count;
                return Ok(count);
            }
            if self.done {
                return Ok(0);
            }
            self.pending.clear();
            self.offset = 0;
            match self.stream.next() {
                Some(Ok(record)) => {
                    self.pending = serde_json::to_vec(&record)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    self.pending.push(b'\n');
                }
                Some(Err(error)) => {
                    self.done = true;
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        error.to_string(),
                    ));
                }
                None => {
                    self.done = true;
                }
            }
        }
    }
}
