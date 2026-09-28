# CLI command architecture

This document records the architecture decisions behind the txBASE command surface.

The command contract, argument table, and option ownership remain in [CLI command reference](cli.md).

## Explicit subcommands

txBASE uses explicit subcommands so the command's responsibility is visible before it opens a path.

The first token selects one responsibility, and nested subcommands group a lifecycle such as `mvcc`, `wal`, `xbf`, or `index`.

Paths and JSON values are positional when the operation has one unambiguous target.

Named options change interpretation or select a bounded policy, such as `--encoding`, `--schema`, `--bind`, `--keep`, or `--keep-rows`.

`raft membership` is a separate peer control-plane command family because it changes remote cluster membership rather than local catalog contents.
Its status command reports one peer's local metrics, while membership mutations target the current leader and voter changes use observed membership state as compare-and-swap input.
See [CLI command reference](cli.md) for the command contract and [Raft consensus design](raft.md) for the peer API boundary.

## Git reference boundary

The command hierarchy is a txBASE design decision based on its own operation and recovery boundaries.

Git's manual describes conventions used by Git, including option and argument rules and how some commands distinguish revisions from paths with `--`.

txBASE uses the manual only to compare Git-specific argument conventions; it is not the authority for txBASE's command-grouping decision.

txBASE does not copy Git's command set, repository model, revision language, object database, or option semantics.

txBASE groups commands around DBF files, catalog directories, sidecars, and local recovery boundaries.
Its command parser does not implement Git's revision language or its revision-versus-path disambiguation.

## Failure-boundary placement

A command belongs to the smallest responsibility group that matches its failure behavior and deterministic fixtures.

Read-oriented commands must state whether normal recovery can change on-disk state.

`wal inspect` is the non-mutating exception because it reports complete records and an incomplete final tail without creating or truncating the WAL.

Mutation commands reuse the lock, recovery, and journal path owned by the DBF, catalog, or XBF operation.

A command that changes only a sidecar must say so in its help and topic document.

## Compatibility boundary

A familiar command name does not claim dBASE, MongoDB, Git, or SQLite compatibility.

The CLI exposes a bounded repository-local subset, and each command document defines its own input limits, recovery behavior, write set, and failure statuses.

## Comparison reference

- [Git command-line interface and conventions](https://git-scm.com/docs/gitcli) (comparison for Git-specific argument conventions)
