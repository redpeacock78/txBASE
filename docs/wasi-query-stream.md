# WASI query streaming

Build the component with the pinned CI command, then run it in Wasmtime with a preopened directory.

## 1. Build and run

CI builds the `wasi-query-stream` example for Rust's `wasm32-wasip2` target.
The example uses `wasip3` to export a WASI 0.3 `wasi:cli/command` component.
This target-specific example requires Rust 1.87 or later; the crate's general minimum remains 1.85.

```bash
cargo build --locked --example wasi-query-stream --target wasm32-wasip2 --release
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  /data/users.dbf '{"projection":{"NAME":1}}'
```

The command prints one JSON object per line.
Wasmtime passes the component filename as argument zero, followed by the DBF path and query JSON.
The `--dir` option grants access to the host directory and maps it to `/data` inside the component.

## 2. Query contract

The component reads one preopened DBF file into memory, then reuses `DbfTable::from_bytes`, `query::parse`, and `query::stream_query`.
It accepts the shared streaming controls: `filter`, `projection`, `skip`, and `limit`.
It rejects `sort`, aggregation, pagination, and cursor controls before writing a row.

The component polls the existing `AsyncQueryStream` and serializes each result as one UTF-8 NDJSON line.
It writes rows to a WASI component-model byte stream and awaits `wasi:cli/stdout.write-via-stream` concurrently.
The stdout consumer therefore applies backpressure to row production.

## 3. Errors and cancellation

Argument, file, DBF, query, and stdout errors are written to stderr and return a failed command result.
Streaming can emit earlier rows before a later row error, so stdout is not an atomic result.
The CI smoke test verifies that a rejected streaming control fails before writing any row.

Dropping the Rust query stream ends row production.
This command does not make the synchronous DBF file read interruptible and does not define host timeout or process-cancellation policy.

## 4. CI boundary

The `wasi-query-stream` CI job installs the WASI target and Wasmtime `49.0.0`, builds the component, and runs `tests/wasi_query_stream_smoke.sh`.
The smoke check decodes the pinned DBF fixture, checks the projected `Alice` row, and verifies that `sort` is rejected without stdout output.

This proves the component build and CLI behavior on the pinned Wasmtime runtime.
It does not prove deployment in another WASI host or provide an `AsyncObjectStore` adapter for XBF snapshots.

## Primary references and scope

- [WASI 0.3 and native async](https://wasi.dev/releases/wasi-p3)
- [`wasip3` 0.9.0 bindings](https://docs.rs/wasip3/0.9.0%2Bwasi-0.3.0/wasip3/)
- [Rust `wasm32-wasip2` target](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html)
- [Wasmtime CLI options](https://docs.wasmtime.dev/cli-options.html)
- [Bytecode Alliance Wasmtime setup action](https://github.com/bytecodealliance/actions)

WASI 0.3.1 is the current release and adds Component Model features beyond the async primitives introduced in WASI 0.3.0.
This adapter uses the 0.3.0 `stream` and `future` boundary and does not depend on the 0.3.1-only WIT features.
