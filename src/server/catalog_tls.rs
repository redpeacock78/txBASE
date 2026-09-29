mod proxy;

use super::CatalogTlsConfig;
use rustls::{
    RootCertStore, ServerConfig, pki_types::CertificateDer, server::WebPkiClientVerifier,
};
use std::{
    fmt::Write as _,
    fs::File,
    io::BufReader,
    net::{SocketAddr, TcpListener},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};
use tiny_http::{Request, Server};
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;

const INTERNAL_HEADER: &str = "x-txbase-internal-listener-token";

pub(super) struct ProtectedHttpListener {
    server: Arc<Server>,
    token: Option<String>,
    tls_worker: Option<TlsWorker>,
    public_addr: SocketAddr,
}

struct TlsWorker {
    shutdown: oneshot::Sender<()>,
    thread: JoinHandle<()>,
}

impl ProtectedHttpListener {
    pub(super) fn bind(bind: &str, config: Option<&CatalogTlsConfig>) -> Result<Self, String> {
        let Some(config) = config else {
            let server =
                Server::http(bind).map_err(|error| format!("cannot bind {bind}: {error}"))?;
            let public_addr = server
                .server_addr()
                .to_ip()
                .ok_or_else(|| format!("HTTP listener {bind} is not an IP socket"))?;
            return Ok(Self {
                server: Arc::new(server),
                token: None,
                tls_worker: None,
                public_addr,
            });
        };

        let tls_config = load_server_config(config)?;
        let public_listener = TcpListener::bind(bind)
            .map_err(|error| format!("cannot bind HTTPS listener {bind}: {error}"))?;
        public_listener
            .set_nonblocking(true)
            .map_err(|error| format!("cannot configure HTTPS listener {bind}: {error}"))?;
        let public_addr = public_listener
            .local_addr()
            .map_err(|error| format!("cannot inspect HTTPS listener {bind}: {error}"))?;

        let server = Server::http("127.0.0.1:0")
            .map_err(|error| format!("cannot bind private HTTP backend: {error}"))?;
        let backend_addr = server
            .server_addr()
            .to_ip()
            .ok_or_else(|| "private HTTP backend is not an IP socket".to_owned())?;
        let server = Arc::new(server);
        let token = new_internal_token()?;
        let worker_token = token.clone();
        let (shutdown, shutdown_rx) = oneshot::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker_server = Arc::clone(&server);
        let acceptor = TlsAcceptor::from(tls_config);
        let thread = thread::Builder::new()
            .name("txbase-https-listener".to_owned())
            .spawn(move || {
                run_tls_worker(
                    public_listener,
                    backend_addr,
                    worker_token,
                    acceptor,
                    shutdown_rx,
                    ready_tx,
                    worker_server,
                );
            })
            .map_err(|error| format!("cannot start HTTPS listener: {error}"))?;

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(Self {
                server,
                token: Some(token),
                tls_worker: Some(TlsWorker { shutdown, thread }),
                public_addr,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                let _ = thread.join();
                Err("HTTPS listener stopped during startup".to_owned())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let _ = shutdown.send(());
                server.unblock();
                let _ = thread.join();
                Err("timed out starting HTTPS listener".to_owned())
            }
        }
    }

    pub(super) fn incoming_requests(&self) -> tiny_http::IncomingRequests<'_> {
        self.server.incoming_requests()
    }

    pub(super) fn public_addr(&self) -> SocketAddr {
        self.public_addr
    }

    pub(super) fn is_tls(&self) -> bool {
        self.tls_worker.is_some()
    }

    pub(super) fn authorizes(&self, request: &Request) -> bool {
        request_has_internal_token(request, self.token.as_deref())
    }

    pub(super) fn unblock(&self) {
        self.server.unblock();
    }
}

fn request_has_internal_token(request: &Request, expected: Option<&str>) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let mut values = request
        .headers()
        .iter()
        .filter(|header| header.field.equiv(INTERNAL_HEADER));
    values
        .next()
        .is_some_and(|header| header.value.as_str() == expected)
        && values.next().is_none()
}

impl Drop for ProtectedHttpListener {
    fn drop(&mut self) {
        if let Some(worker) = self.tls_worker.take() {
            let _ = worker.shutdown.send(());
            self.server.unblock();
            let _ = worker.thread.join();
        }
    }
}

pub(super) fn load_server_config(config: &CatalogTlsConfig) -> Result<Arc<ServerConfig>, String> {
    let certificates = read_certificates(&config.certificate, "server certificate")?;
    let private_key = rustls_pemfile::private_key(&mut BufReader::new(
        File::open(&config.private_key)
            .map_err(|error| format!("cannot read TLS private key: {error}"))?,
    ))
    .map_err(|error| format!("cannot parse TLS private key: {error}"))?
    .ok_or_else(|| "TLS private key file contains no private key".to_owned())?;

    let server_config = if let Some(path) = &config.client_ca {
        let ca_certificates = read_certificates(path, "client CA certificate")?;
        let mut roots = RootCertStore::empty();
        for certificate in ca_certificates {
            roots
                .add(certificate)
                .map_err(|error| format!("cannot add client CA certificate: {error}"))?;
        }
        let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
            .build()
            .map_err(|error| {
                format!("cannot configure client certificate verification: {error}")
            })?;
        ServerConfig::builder()
            .with_client_cert_verifier(verifier)
            .with_single_cert(certificates, private_key)
            .map_err(|error| format!("invalid TLS certificate or private key: {error}"))?
    } else {
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certificates, private_key)
            .map_err(|error| format!("invalid TLS certificate or private key: {error}"))?
    };

    let mut server_config = server_config;
    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(Arc::new(server_config))
}

fn read_certificates(
    path: &std::path::Path,
    label: &str,
) -> Result<Vec<CertificateDer<'static>>, String> {
    let file = File::open(path).map_err(|error| format!("cannot read {label}: {error}"))?;
    let certificates = rustls_pemfile::certs(&mut BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot parse {label}: {error}"))?;
    if certificates.is_empty() {
        return Err(format!("{label} file contains no certificates"));
    }
    Ok(certificates)
}

fn new_internal_token() -> Result<String, String> {
    let mut random = [0_u8; 32];
    getrandom::fill(&mut random)
        .map_err(|error| format!("cannot generate private HTTP listener token: {error}"))?;
    let mut token = String::with_capacity(random.len() * 2);
    for byte in random {
        write!(&mut token, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(token)
}

fn run_tls_worker(
    public_listener: TcpListener,
    backend_addr: SocketAddr,
    token: String,
    acceptor: TlsAcceptor,
    shutdown: oneshot::Receiver<()>,
    ready: mpsc::SyncSender<Result<(), String>>,
    server: Arc<Server>,
) {
    // ponytail: keep proxy runtime resource use fixed; add workers if TLS throughput requires it.
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(format!(
                "cannot create HTTPS listener runtime: {error}"
            )));
            server.unblock();
            return;
        }
    };
    let listener = {
        let _runtime = runtime.enter();
        tokio::net::TcpListener::from_std(public_listener)
    };
    let listener = match listener {
        Ok(listener) => listener,
        Err(error) => {
            let _ = ready.send(Err(format!("cannot initialize HTTPS listener: {error}")));
            server.unblock();
            return;
        }
    };
    if ready.send(Ok(())).is_err() {
        server.unblock();
        return;
    }
    if let Err(error) = runtime.block_on(proxy::run(
        listener,
        backend_addr,
        Arc::from(token),
        acceptor,
        shutdown,
    )) {
        eprintln!("HTTPS listener stopped: {error}");
        server.unblock();
    }
}

pub(super) const fn internal_header_name() -> &'static str {
    INTERNAL_HEADER
}

#[cfg(test)]
#[path = "catalog_tls_tests.rs"]
pub(in crate::server) mod tests;
