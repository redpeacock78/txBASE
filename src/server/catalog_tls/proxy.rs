use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::{
    Request, Response, Uri, Version,
    body::Incoming,
    header::{
        CONNECTION, HOST, HeaderMap, HeaderName, HeaderValue, PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION, TE, TRAILER, TRANSFER_ENCODING, UPGRADE, VIA,
    },
    server::conn::http1,
    service::service_fn,
};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use std::{convert::Infallible, error::Error, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::oneshot, task::JoinSet, time::timeout};
use tokio_rustls::TlsAcceptor;

use super::internal_header_name;

type ProxyError = Box<dyn Error + Send + Sync>;
type ProxyBody = UnsyncBoxBody<Bytes, ProxyError>;
type ProxyClient = Client<HttpConnector, ProxyBody>;

const TLS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

pub(super) async fn run(
    listener: TcpListener,
    backend_addr: SocketAddr,
    token: Arc<str>,
    acceptor: TlsAcceptor,
    mut shutdown: oneshot::Receiver<()>,
) -> Result<(), String> {
    let mut connector = HttpConnector::new();
    connector.enforce_http(true);
    connector.set_nodelay(true);
    let client: ProxyClient = Client::builder(TokioExecutor::new()).build(connector);
    let mut connections = JoinSet::new();

    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|error| format!("HTTPS accept failed: {error}"))?;
                let acceptor = acceptor.clone();
                let client = client.clone();
                let token = Arc::clone(&token);
                connections.spawn(async move {
                    let Ok(Ok(stream)) = timeout(TLS_HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await else {
                        return;
                    };
                    let service = service_fn(move |request| {
                        forward_request(client.clone(), backend_addr, Arc::clone(&token), request)
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            _ = &mut shutdown => break,
        }
    }

    connections.abort_all();
    while connections.join_next().await.is_some() {}
    Ok(())
}

async fn forward_request(
    client: ProxyClient,
    backend_addr: SocketAddr,
    token: Arc<str>,
    request: Request<Incoming>,
) -> Result<Response<ProxyBody>, Infallible> {
    match forward(client, backend_addr, token, request).await {
        Ok(response) => Ok(response),
        Err(()) => Ok(Response::builder()
            .status(502)
            .body(full_body("HTTP backend unavailable"))
            .expect("static gateway response is valid")),
    }
}

async fn forward(
    client: ProxyClient,
    backend_addr: SocketAddr,
    token: Arc<str>,
    request: Request<Incoming>,
) -> Result<Response<ProxyBody>, ()> {
    let (mut parts, body) = request.into_parts();
    let path_and_query = parts
        .uri
        .path_and_query()
        .cloned()
        .unwrap_or_else(|| hyper::http::uri::PathAndQuery::from_static("/"));
    let uri = Uri::builder()
        .scheme("http")
        .authority(backend_addr.to_string())
        .path_and_query(path_and_query)
        .build()
        .map_err(|_| ())?;

    strip_hop_by_hop(&mut parts.headers);
    parts.headers.remove(HOST);
    parts.headers.remove(internal_header_name());
    parts.headers.insert(
        HeaderName::from_static(internal_header_name()),
        HeaderValue::from_bytes(token.as_bytes()).map_err(|_| ())?,
    );
    append_via(&mut parts.headers);
    parts.uri = uri;
    parts.version = Version::HTTP_11;

    let body = body
        .map_err(|error| Box::new(error) as ProxyError)
        .boxed_unsync();
    let request = Request::from_parts(parts, body);
    let response = client.request(request).await.map_err(|_| ())?;
    let (mut parts, body) = response.into_parts();
    strip_hop_by_hop(&mut parts.headers);
    append_via(&mut parts.headers);
    let body = body
        .map_err(|error| Box::new(error) as ProxyError)
        .boxed_unsync();
    Ok(Response::from_parts(parts, body))
}

fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let connection_fields = headers
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
        .collect::<Vec<_>>();
    for name in connection_fields {
        headers.remove(name);
    }
    for name in [
        CONNECTION,
        HeaderName::from_static("proxy-connection"),
        HeaderName::from_static("keep-alive"),
        TE,
        TRAILER,
        TRANSFER_ENCODING,
        UPGRADE,
        PROXY_AUTHENTICATE,
        PROXY_AUTHORIZATION,
    ] {
        headers.remove(name);
    }
}

fn append_via(headers: &mut HeaderMap) {
    headers.append(VIA, HeaderValue::from_static("1.1 txbase"));
}

fn full_body(value: &'static str) -> ProxyBody {
    Full::new(Bytes::from_static(value.as_bytes()))
        .map_err(|never: Infallible| -> ProxyError { match never {} })
        .boxed_unsync()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_connection_nominated_and_standard_hop_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONNECTION,
            HeaderValue::from_static("x-private, keep-alive"),
        );
        headers.insert("x-private", HeaderValue::from_static("hidden"));
        headers.insert(
            HeaderName::from_static("keep-alive"),
            HeaderValue::from_static("timeout=5"),
        );
        headers.insert(TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        headers.insert(TRAILER, HeaderValue::from_static("x-checksum"));
        headers.insert("x-end-to-end", HeaderValue::from_static("kept"));

        strip_hop_by_hop(&mut headers);

        assert!(!headers.contains_key(CONNECTION));
        assert!(!headers.contains_key("x-private"));
        assert!(!headers.contains_key("keep-alive"));
        assert!(!headers.contains_key(TRAILER));
        assert!(!headers.contains_key(TRANSFER_ENCODING));
        assert_eq!(headers["x-end-to-end"], "kept");
    }

    #[test]
    fn appends_proxy_to_existing_via_chain() {
        let mut headers = HeaderMap::new();
        headers.insert(VIA, HeaderValue::from_static("1.0 upstream"));

        append_via(&mut headers);

        assert_eq!(headers.get_all(VIA).iter().count(), 2);
        assert_eq!(headers.get_all(VIA).iter().nth(1).unwrap(), "1.1 txbase");
    }
}
