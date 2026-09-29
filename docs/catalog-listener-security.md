# Public catalog listener transport security

Status: `serve-catalog` supports opt-in server TLS and optional mutual TLS (mTLS).

This contract covers the public listener in both fixed-term replication mode and Raft mode.
It does not configure the separate Raft peer listener.

## TLS configuration

Pass `--tls-cert PEM` and `--tls-key PEM` together to serve the catalog over HTTPS.
Without those options, `serve-catalog` keeps its existing HTTP behavior.
The server loads the certificate chain and private key at startup, verifies that they match, and advertises HTTP/1.1 through ALPN.
TLS uses Rustls defaults with TLS 1.2 and TLS 1.3 enabled.

Pass `--tls-client-ca PEM` with the server certificate and key to require a client certificate issued by one of the listed trusted CA certificates.
The listener rejects clients that omit a certificate or present a chain it cannot validate.
If `--tls-client-ca` is omitted, clients do not need a certificate.
All certificates accepted through the configured CA set receive the same catalog API access; txBASE does not map certificate identities to roles or check certificate revocation lists.

TLS files are read once during startup.
Replace a certificate or trust set by restarting the server.
Invalid TLS files and listener-bind errors are reported before txBASE opens the catalog.

The public `--tls-cert`, `--tls-key`, and `--tls-client-ca` options are independent of the Raft peer options `--raft-peer-cert`, `--raft-peer-key`, and `--raft-peer-client-ca`.
The Raft options configure only the peer listener; see [Raft consensus design](raft.md).

## HTTP forwarding boundary

The catalog route handlers continue to use `tiny_http`.
Because its [`SslConfig`](https://docs.rs/tiny_http/0.12.0/tiny_http/struct.SslConfig.html) accepts a certificate and private key but does not expose a client-certificate verifier, a Rustls frontend terminates public TLS and forwards HTTP/1.1 to a `tiny_http` backend bound to an ephemeral loopback port.
Rustls [`WebPkiClientVerifier`](https://docs.rs/rustls/0.23.45/rustls/server/struct.WebPkiClientVerifier.html) requires and validates client certificates when a client CA is configured.

The frontend streams request and response bodies instead of buffering complete messages.
It removes fields named by `Connection` and the known hop-by-hop fields before forwarding, then appends `Via` to the forwarded request and response as required for an HTTP intermediary by [RFC 9110 section 7.6.1](https://www.rfc-editor.org/rfc/rfc9110.html#section-7.6.1) and [section 7.6.3](https://www.rfc-editor.org/rfc/rfc9110.html#section-7.6.3).
The implementation uses [`tokio-rustls`](https://docs.rs/tokio-rustls/0.26.6/tokio_rustls/server/struct.TlsAcceptor.html) for TLS and Hyper's [HTTP/1 server connection](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html), [streaming bodies](https://docs.rs/hyper/1.11.1/hyper/body/index.html), and [pooled HTTP client](https://docs.rs/hyper-util/0.1.21/hyper_util/client/legacy/struct.Client.html) for forwarding.

The backend requires a process-generated 256-bit token in a private request header.
The frontend replaces any client-supplied value before forwarding.
Requests sent directly to the loopback backend without that token receive `403 Forbidden`.
This prevents local HTTP access from bypassing the public listener's client-certificate check.

## Client and application authentication

TLS protects the connection and authenticates the server to clients.
mTLS additionally authenticates that each client certificate chains to the configured CA set.
It does not assign different catalog permissions to different certificates.

`TXBASE_REPLICATION_TOKEN` remains a separate Bearer credential for replication routes.
It is not a substitute for TLS because HTTP exposes it to observers on the network.
The built-in `replicate catch-up` client can present a client certificate when given `--tls-client-cert` and `--tls-client-key` together.
The client verifies the server with the operating system's trust facilities and requires HTTPS when a client identity is configured.

Raft peer TLS and public catalog TLS have separate listeners and trust configuration.
Raft peer mTLS is enabled independently with `--raft-peer-client-ca`; see [Raft consensus design](raft.md).

## CLI examples

Server-authenticated HTTPS:

```sh
txbase serve-catalog ./data --bind 0.0.0.0:8443 \
  --tls-cert ./server-chain.pem --tls-key ./server-key.pem
```

HTTPS with required client certificates:

```sh
txbase serve-catalog ./data --bind 0.0.0.0:8443 \
  --tls-cert ./server-chain.pem --tls-key ./server-key.pem \
  --tls-client-ca ./client-ca.pem
```

The built-in catch-up client can connect to that listener with its own identity:

```sh
txbase replicate catch-up ./follower https://authority.example \
  --replication-term 4 --follower-id follower-1 \
  --tls-client-cert ./follower-chain.pem --tls-client-key ./follower-key.pem
```

The same public TLS options apply when `serve-catalog` uses `--raft-*` options.
Keep private keys readable only by the server account and use a CA dedicated to the clients that should reach this catalog.
