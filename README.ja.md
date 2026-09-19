# txBASE

txBASEは、dBASEのDBF file formatを中心に据えたRust製database prototypeです。

選択したdBASEとVisual FoxProのfieldを読み書きし、JSONとHTTPのinterfaceを提供します。

pathから読み込んだmutationにはfile-backed WALを使い、DBFとmemo sidecarの保存を復旧可能にします。

英語のREADMEは[README.md](README.md)です。

## 使い方

### 最短の手順

DBFをJSONとして読み取ります。

```bash
cargo run -- path/to/users.dbf
```

HTTP serverを起動します。

```bash
cargo run -- --serve path/to/users.dbf
```

default listenerは`127.0.0.1:8080`です。

listener addressは`--bind ADDRESS`で変更できます。

### 読み取り

```bash
curl -s http://127.0.0.1:8080/records | jq
curl -s http://127.0.0.1:8080/records/1 | jq
```

responseのrecord numberは、DBFのphysical record numberを1-basedで表します。

### Query

`QUERY /records`は`application/json`のquery documentを受け付けます。

```bash
curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"limit":10}' \
  http://127.0.0.1:8080/records
```

現在のqueryは`filter`、`sort`、`projection`、`skip`、`limit`を提供します。

predicateは`$eq`、`$ne`、`$gt`、`$gte`、`$lt`、`$lte`、`$in`、`$nin`、`$and`、`$or`、`$not`です。

HTTP境界の詳細は[HTTP method semantics](docs/http-semantics.md)を参照してください。

### Mutation

`POST`はrecordを作成します。

`PUT`はrecord全体を置き換えます。

`PATCH`はrecordを部分更新し、`$set`、`$unset`、`$inc`を使えます。

`DELETE`はDBFのdeletion markerを設定します。

path-loaded mutationは、可能なら`TXDP` byte-range deltaを使います。

それ以外の場合は`TXDB`または`TXDM` snapshotを使います。

WALをsyncしてからDBFまたはmemo sidecarを置き換えます。

起動時には未完了のmutationを復旧します。

### Inspectとmaintenance

schemaとverifyはDBFを読み取り、schemaとrecord boundaryを確認します。

```bash
txbase schema path/to/users.dbf
txbase verify path/to/users.dbf
txbase catalog path/to/database
txbase verify-catalog path/to/database
txbase index build path/to/users.dbf NAME AGE
txbase index verify path/to/users.dbf
txbase index rebuild path/to/users.dbf
txbase pack path/to/users.dbf
txbase recall path/to/users.dbf 2
```

`pack`はlogical delete済みrecordを物理的に除去し、残ったrecord numberを詰め直します。

`recall`はphysical record numberを指定してlogical deleteを取り消します。

`catalog`はdirectory直下のDBF tableを発見し、各tableのschemaを表示します。

`verify-catalog`は発見したすべてのtableをverifyします。

`index build`はscalar keyのexternal sidecarを作成します。

`index verify`はDBFまたはmemo sidecarが変更されたindexをstaleとして拒否します。

`index rebuild`はtable mutation後にindex entryを再生成します。

backupとrestoreは、DBFと同じstemの`.dbt`または`.fpt` sidecarもコピーします。

```bash
txbase backup path/to/users.dbf backups/users.dbf
txbase restore backups/users.dbf path/to/users.dbf
```

## Install

Rust stableを[rustup](https://rustup.rs/)でinstallします。

```bash
cargo build --release
```

CIはUbuntu、macOS、Windowsで同じquality gateを実行します。

## Development

### Quality gate

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

### 構成

```text
src/dbf/            DBF parser、codec、memo sidecar、maintenance、mutation、WAL、test
src/catalog.rs      directory直下のDBF発見、table lookup、catalog verify
src/index.rs        scalar keyのexternal index sidecar lifecycle
src/query.rs        JSON queryの実行とvalidation
src/query_path.rs   dotted pathとprojectionのhelper
src/server.rs       HTTP routing、QUERY validation、DBF mutation
src/transaction.rs  fileまたはmemory WALとsnapshot transaction
tests/fixtures/     外部formatのfixture
tests/corpus/       malformed inputのcorpus
docs/               仕様調査と設計判断
```

fileは責務と保守性が明確になる粒度で分割します。

crate分割はbuildまたはownershipの境界が必要になるまで行いません。

### 詳細文書

- [DBFとdBASE compatibility](docs/dbf-compatibility.md)
- [Multi-table catalog](docs/catalog.md)
- [Secondary-index sidecar](docs/indexes.md)
- [MongoDB query model](docs/query-model.md)
- [Firebase data model](docs/firebase-model.md)
- [SQLite testingとquality](docs/testing-quality.md)
- [HTTP semanticsとQUERY](docs/http-semantics.md)
- [Roadmapと明示的なnon-goal](docs/roadmap.md)
- [Research index](docs/research.md)

## Todo

当面はDBF、memo、WAL、query、HTTPのcontractをfixtureとfailure testで固めます。

secondary indexの自動更新とquery planner、join、aggregation、cursor、multi-record transaction、CJK encoding、XBFはroadmapで検討します。

## License

MITです。
