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

現在のqueryは`filter`、`sort`、`projection`、`skip`、`limit`、`page_size`、`cursor`、限定された一段の`aggregate`を提供します。

`page_size`を指定すると、physical record順のページを`records`と`cursor`で返します。
次のページでは同じ`page_size`と返却された`cursor`を送ります。
このモードでは現時点で`sort`と`skip`を併用できず、最大1,000件です。
physical cursor pageは、要求されたpageとlook-aheadのmatchを見つけた時点でscanを止めます。
sorted keyset cursorとpublicなstreamingまたはbackpressure APIは未実装です。

`aggregate`は一つの`$group` stageに限定し、`$count`と整数`$sum`を使えます。
`sort`、`projection`、pagination、`limit`との併用はできません。

libraryにはcatalog table二つを読むboundedな`inner`または`left` equality joinもあります。
添付仕様の`from`、`join.on`、`filter`、`projection`を受け付け、qualified keyのJSONを返します。
resultは最大100,000行です。
single-table HTTP serverからはまだ利用できません。

`$expr`による同一record内のfield比較も、二つのscalar operandに限定して提供します。

predicateは`$eq`、`$ne`、`$gt`、`$gte`、`$lt`、`$lte`、`$in`、`$nin`、`$and`、`$or`、`$not`、限定された`$expr`です。

HTTP境界の詳細は[HTTP method semantics](docs/http-semantics.md)を参照してください。

### Mutation

`POST`はrecordを作成します。

`PUT`はrecord全体を置き換えます。

`PATCH`はrecordを部分更新し、`$set`、`$unset`、`$inc`を使えます。

`DELETE`はDBFのdeletion markerを設定します。

複数のrecord mutationは`POST /transaction`で一つのDBFに対してまとめてcommitできます。
全operationをprivate copyに適用してから、一回のsnapshot/WAL boundaryで保存します。
operationが失敗した場合はcopyを破棄し、元のDBFを変更しません。
cross-table atomicityとMVCC visibilityは未実装です。

path-loaded mutationは、可能なら`TXDP` byte-range deltaを使います。

それ以外の場合は`TXDB`または`TXDM` snapshotを使います。

WALをsyncしてからDBFまたはmemo sidecarを置き換えます。

起動時には未完了のmutationを復旧します。

既存index sidecarがある場合、その更新先も同じWALに記録し、crash後にWALを消す前に再適用します。

### Inspectとmaintenance

schemaとverifyはDBFを読み取り、schemaとrecord boundaryを確認します。

DBF headerのlanguage-driver byteが信頼できない場合は、read、schema、verify、pack、recall、serverで`--encoding NAME`を指定できます。

対応するaliasは`windows-31j`/`cp932`、`gbk`/`cp936`、`euc-kr`/`cp949`、`big5`/`cp950`です。

invocation単位のoverrideは`*.txschema.json`より優先され、保存されません。

effectiveな値はschema outputの`encoding_override`で確認できます。

```bash
txbase schema path/to/users.dbf
txbase verify path/to/users.dbf
txbase schema path/to/users.dbf --encoding cp932
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

通常のDBF保存は既存sidecarの更新先をWAL経由で適用し、staleまたはinvalidなsidecarは拒否します。

`index rebuild`は明示的な修復手段です。

path-aware plannerは、複数のsingle-field indexが有効なdirect equality filterであれば候補recordをintersectionできます。

catalog joinは`txbase::query::join::parse`と`execute`から使います。
複数join、cost-based planner、streaming、cross-table transactionは未実装です。

backupとrestoreは、DBFと同じstemの`.dbt`または`.fpt` sidecarもコピーします。

DBF codecはVisual FoxProのCJK driver IDであるWindows-31J/CP932、GBK/CP936、EUC-KR/CP949、Big5/CP950に対応します。
malformed readはU+FFFDにし、unmappableまたはbyte width超過のwriteは拒否します。

optionalな`users.txschema.json` sidecarは、legacy DBF byteを変更せずに一つのfieldへ`primary`、`unique`、`not_null`を設定します。
同じsidecarで対応済みのCJK codecを明示的に選択できます。
load済みのactive recordとinsert、replace、patch、recallをconstraintで検証します。
`CHECK`、foreign key、default、composite keyは未実装です。

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
- [Quality contract matrix](docs/quality-matrix.md)
- [HTTP semanticsとQUERY](docs/http-semantics.md)
- [Schema metadataとlocal constraint](docs/schema-metadata.md)
- [XBF v1 format draft](docs/xbf.md)
- [Roadmapと明示的なnon-goal](docs/roadmap.md)
- [Research index](docs/research.md)

## Todo

当面はDBF、memo、WAL、query、HTTPのcontractをfixtureとfailure testで固めます。

secondary index sidecarの自動更新と単純なequality、uniform selectivity estimateによるequality intersection、histogram estimateによるsingle-field range planner、single-field ordered planner、multi-key sortのordered-prefix planner、fieldごとのdirectionを持つcompound indexによるmulti-key sort planner、equality prefixを使った候補数比較は実装済みです。
一つのfieldに対する`primary`、`unique`、`not_null`のschema metadataも実装済みです。
full cost model、collation-aware planning、sorted query cursor、streaming、複数stageのaggregation、cross-table constraint、CJK encodingの拡張、XBF実装はroadmapで検討します。
XBFのwire contractは[XBF v1 format draft](docs/xbf.md)に記載し、codecはまだ提供していません。

## License

MITです。
