# WASM and worker host boundary

This document isolates the WASM and edge-runtime boundary.

The repository now contains a host-independent DBF core slice. Worker and
asynchronous-storage adapters remain future work.

## 0. Current implementation slice

`wasm::WasmCore` owns an in-memory `DbfTable` and reuses the native parser,
query validator, query executor, and mutation methods.

Its versioned boundary currently provides:

- `ABI_VERSION = 1`;
- `open_dbf` and `snapshot` for byte-in/byte-out DBF state;
- `query_json` for the existing bounded query document;
- `apply_operation_json` for the existing `POST`, `PUT`, `PATCH`, and `DELETE`
  operation IR;
- a `wasm-bindgen` `WasmDatabase` wrapper on `wasm32` with the same methods;
- a native contract test and a CI `wasm32-unknown-unknown` library check.

The core does not write files, access a network, schedule tasks, or commit a
transaction. The host must persist the returned snapshot and provide
serialization, retry, and concurrency control.

## 1. Core boundary

The txBASE core should remain deterministic and mostly independent of its host.

The host should provide HTTP, asynchronous storage, clocks, and platform-specific APIs.

The core should own format codecs, query semantics, mutation rules, WAL encoding, and transaction state transitions.

## 2. Host and core split

A candidate arrangement is:

```text
JavaScript or TypeScript host
        |
        ├─ HTTP
        ├─ asynchronous object storage
        ├─ platform APIs
        ↓
     txbase.wasm
        |
        ├─ DBF and XBF codecs
        ├─ query engine
        ├─ mutation semantics
        ├─ WAL codec
        └─ transaction state machine
```

The host must not make the core depend on POSIX files or WASI-specific behavior.

## 3. Reused contracts

The WASM boundary should reuse the existing DBF and XBF codecs where supported.

It should expose the same bounded query, mutation, validation, and recovery semantics as the native library.

It should not introduce a second query language, a second transaction model, or a host-specific interpretation of DBF bytes.

The object-store contract belongs below the shared table and transaction interfaces.

## 4. Target hosts

The same core could eventually run behind:

- native Rust APIs;
- Cloudflare Workers or another worker runtime;
- browser storage such as OPFS;
- Node, Deno, or Bun adapters;
- WASI-compatible runtimes;
- another host that supplies the required storage primitives.

The list is a compatibility target, not a promise that every host will be supported.

## 5. Host services

Before implementation, the boundary must define:

- asynchronous range reads and writes;
- immutable object publication;
- conditional manifest updates;
- bounded memory and payload limits;
- cancellation and timeout behavior;
- error and retry mapping;
- deterministic clock or generation inputs for tests.

The host owns scheduling and platform I/O.

The core owns the meaning of a committed database state.

## 6. Acceptance conditions

The current core slice meets the following initial conditions:

- a versioned host-neutral ABI;
- one native host fixture;
- identical query and mutation implementation paths across native and WASM;
- explicit malformed-input errors at the byte and JSON boundaries.

The following conditions remain before calling a worker or WASI host complete:

- one worker or WASI runtime smoke test;
- explicit storage, timeout, and cancellation error mapping;
- an asynchronous object-store adapter with conditional publication.

Until then, WASM hosting is a current core boundary with future host adapters.

## 7. Explicit non-goals

This document does not promise a JavaScript ORM, a browser-only database, or a POSIX emulation layer inside WASM.

Those would be separate products and would obscure the shared core contract.

## Primary references and scope

- [WebAssembly Core Specification](https://webassembly.github.io/spec/core/)
- [WASI](https://wasi.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [Cloudflare Workers WebAssembly](https://developers.cloudflare.com/workers/runtime-apis/webassembly/)
- [Node.js WASI](https://nodejs.org/api/wasi.html)

The WebAssembly and WASI specifications define the core module and host-interface vocabulary.
The Component Model and the Cloudflare Workers and Node.js pages are implementation references for possible hosts, not txBASE compatibility commitments.

The repository has a WASM core implementation and target compile check, but it does not claim that a worker or WASI runtime, asynchronous storage, or native recovery path is already supported.
