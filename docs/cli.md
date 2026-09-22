# CLI command design

The CLI is a stable inspection and maintenance boundary over the Rust APIs.

It uses explicit subcommands so a command's read, write, recovery, or server responsibility is visible before a path is opened.

## Command shape

The top-level form is:

```text
txbase COMMAND [SUBCOMMAND] ARGUMENT...
```

The first token selects one responsibility. Nested subcommands are used when a family has its own lifecycle, such as `mvcc`, `wal`, `xbf`, and `index`.

Paths and JSON values are positional where the operation has one unambiguous target.
Options are named when they change interpretation, such as `--encoding`, `--schema`, `--bind`, or `--keep`.

## Responsibility groups

| Group | Commands | Boundary |
| --- | --- | --- |
| Read and inspect | `read`, `cdc`, `schema`, `verify`, `catalog`, `verify-catalog`, `wal inspect`, `mvcc list`, `mvcc read`, `mvcc row`, `mvcc row-at`, `mvcc catalog list`, `mvcc catalog read`, `xbf report`, `index verify` | Read-only output; these commands do not intentionally publish a mutation. |
| Create and mutate | `init`, `insert`, `pack`, `recall`, `schema apply`, `mvcc gc`, `mvcc catalog gc`, `index build`, `index build-compound`, `index rebuild`, `xbf import`, `xbf export` | May write DBF bytes, sidecars, or durable history according to the command contract. |
| Copy and serve | `backup`, `restore`, `serve`, `serve-catalog` | Copy or expose data through a separately documented boundary. |

`schema apply` is deliberately separate from `schema`: `schema` inspects the current metadata, while `schema apply` validates and installs a candidate sidecar.

`cdc` is a read-only event inspection command, not a mutation or consumer acknowledgement command.

Its optional `--after` argument is an exclusive transaction-ID cursor over the committed events in the table's CDC sidecar.

## Design rules

- Read-only inspection commands must not create or truncate a file merely to inspect it.
- Mutation commands must reuse the DBF, catalog, or XBF lock and recovery path that owns the target.
- A command that changes a sidecar without changing DBF bytes must say so explicitly in its help and topic document.
- A new command belongs in the smallest responsibility group that matches its failure behavior and fixtures.
- The CLI may expose a bounded repository-local subset; a familiar command name does not claim dBASE, MongoDB, Git, or SQLite compatibility.

The command surface follows the explicit subcommand and option conventions described by the [Git command-line interface documentation](https://git-scm.com/docs/gitcli), but txBASE does not copy Git's command set, repository model, or option semantics.

## Scope

This document defines the txBASE CLI surface and design rationale.
Database consistency, DBF compatibility, XBF recovery, MVCC retention, and HTTP semantics remain in their topic documents.
