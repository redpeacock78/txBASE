# WASM and worker host boundary

This document isolates the future WASM and edge-runtime boundary.

The design is not a current txBASE feature.

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

An initial WASM slice is complete only when it has:

- a versioned host ABI;
- one native host fixture;
- one worker or WASI smoke test;
- identical query and mutation results across the native and WASM paths;
- explicit handling for storage, timeout, cancellation, and malformed-input errors.

Until then, WASM hosting remains future work.

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

There is no implementation or CI path for WASM in the current repository, so this document makes no claim that the native codec, query, mutation, or recovery behavior already runs in a WASM host.
