# CLI command architecture

This document records the architecture decisions behind the txBASE command surface.

The command contract, argument table, and option ownership remain in [CLI command reference](cli.md).

## Explicit subcommands

txBASE uses explicit subcommands so the command's responsibility is visible before it opens a path.

The first token selects one responsibility, and nested subcommands group a lifecycle such as `mvcc`, `wal`, `xbf`, or `index`.

Paths and JSON values are positional when the operation has one unambiguous target.

Named options change interpretation or select a bounded policy, such as `--encoding`, `--schema`, `--bind`, `--keep`, or `--keep-rows`.

## Git reference boundary

The Git command-line interface documentation is a reference for explicit subcommands and named options.

txBASE does not copy Git's command set, repository model, revision language, object database, or option semantics.

The resemblance is structural only: txBASE operates on DBF files, catalog directories, sidecars, and local recovery boundaries rather than Git repositories.

## Failure-boundary placement

A command belongs to the smallest responsibility group that matches its failure behavior and deterministic fixtures.

Read-oriented commands must state whether normal recovery can change on-disk state.

`wal inspect` is the non-mutating exception because it reports complete records and an incomplete final tail without creating or truncating the WAL.

Mutation commands reuse the lock, recovery, and journal path owned by the DBF, catalog, or XBF operation.

A command that changes only a sidecar must say so in its help and topic document.

## Compatibility boundary

A familiar command name does not claim dBASE, MongoDB, Git, or SQLite compatibility.

The CLI exposes a bounded repository-local subset, and each command document defines its own input limits, recovery behavior, write set, and failure statuses.

## Primary reference

- [Git command-line interface and conventions](https://git-scm.com/docs/gitcli)
