# txBASE

txBASEは、dBASEのDBF file formatを中心に据えたRust製のdatabase prototypeです。

選択したdBASEとVisual FoxProのfieldを読み書きし、JSONとHTTPのinterfaceを提供します。

pathから読み込んだmutationにはfile-backed WALを使い、DBFとmemo sidecarの保存を復旧可能にします。

英語のREADMEは[README.md](README.md)です。

日本語ドキュメントの入口は[docs/ja/README.md](docs/ja/README.md)です。

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

複数DBFのjoinをHTTPから使う場合は、catalog serverを起動します。

```bash
cargo run -- --serve-catalog path/to/database
```

`GET /catalog`でschema、`GET`/`HEAD /{table}/records[/{id}]`でnamed tableを読み取れます。
`POST /{table}/records`と`PUT`/`PATCH`/`DELETE /{table}/records/{id}`は、single-table serverと同じWAL/ETag semanticsで一つのDBFを更新します。
`QUERY /{table}/records`と`QUERY /{table}/explain`はsingle-table serverと同じquery documentを受け付けます。`QUERY /{table}/records/stream`はsingle-tableのstream routeと同じfilter、projection、skip、limitを有界NDJSONで返します。`QUERY /join`はbounded joinを返します。named-table mutationはsingle-tableと同じ`X-Txbase-Transaction-Id`を返します。catalog serverの`POST /transaction`はnamed-table mutationをcatalog journalで複数DBFへatomicにcommitし、durableなcatalog transaction IDをJSONと`X-Txbase-Transaction-Id`で返します。historical row versionとMVCC visibilityは未実装です。

single-table serverの`QUERY /explain`は、同じquery documentに対するtable scanまたはindex
planを構造化JSONで返します。

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

現在のqueryは`filter`、`sort`、`projection`、`collation`、`skip`、`limit`、`page_size`、`cursor`、限定された`aggregate`を提供します。

`page_size`を指定すると、physical record順のページを`records`と`cursor`で返します。
次のページでは同じ`page_size`と返却された`cursor`を送ります。
`sort`を指定した場合は、sort keyとphysical record numberを含むkeyset cursorになります。
どちらのcursor modeも`skip`とは併用できず、最大1,000件です。
physical cursor pageは、要求されたpageとlook-aheadのmatchを見つけた時点でscanを止めます。
sorted cursor pageは既存のsort comparatorを再利用します。
新しく発行するcursorはopaqueなversion付きtokenで、発行時のtable representationに束縛されます。
table変更後に再利用すると、snapshotを混ぜずにinvalid-queryとして拒否します。
旧numeric physical cursorとversion-1 sorted cursorは互換のため受け入れますが、snapshot束縛はありません。
libraryの`query::stream_query`は、matching record全体をmaterializeせずにfilter、projection、skip、limitを適用するborrowed iteratorです。
sort、aggregate、page-size、cursorはblockingまたはresume boundaryを必要とするため拒否します。
`query::stream_query_snapshot`はloaded tableのcloneを保持するため、元tableへの後続mutationから独立したpull-based iteratorです。
`query::stream_query_bounded`はそのsnapshot iteratorをboundedな標準library channelの背後で動かし、consumerが読み取らない間はproducerを停止します。consumerをdropするとproducerも停止します。`QUERY /records/stream`はこのstreamを`application/x-ndjson`で返し、HTTP/1.1ではchunked transferを使います。runtime固有のasync traitは未実装です。

`aggregate`は、0個以上の`$match` stageの後に一つの終端`$count`または`$distinct` stage、または一つの`$group` stageを使えます。
`$group`の後には有界な`$match` stageを置き、その後に一つの`$project`、最後の`$sort`、最後の`$limit`を置けます。
`$count`、数値の`$sum`、数値の`$avg`、および比較可能な値に対する`$min` / `$max`をサポートします。
`$avg`はmissing、null、非数値を無視し、group内に数値がなければ`null`を返します。
`$sum`はmissing、null、非数値を無視し、入力がすべて整数なら整数を返し、累積結果が有限なJSON数値でなければ拒否します。
`$project`はgroup出力に対してinclude/exclude projectionを適用し、`$sort`または`$limit`の前に置きます。
top-levelの`sort`、`projection`、pagination、`limit`との併用はできません。

libraryにはcatalog table二つ以上を読むboundedな`inner`、`left`、`right`、`semi`、`anti` equality joinと`cross` joinがあります。
添付仕様の`from`、`join.on`、`filter`、`projection`を受け付け、qualified keyのJSONを返します。
resultと各中間stageは最大100,000行です。
`semi`と`anti`は右側の列を返さず、右側のmatch有無で左側の行を一度だけ返します。
`cross`は空の`on`を受け付け、候補pair数を100,000以下に制限します。
single-table HTTP serverからは利用できませんが、`--serve-catalog DIRECTORY`のcatalog serverでは
catalog schemaを`GET /catalog`でstrongなcatalog representation `ETag`とともに取得し、read-onlyな`QUERY /join`として利用できます。
cross-table mutationは`POST /transaction`でatomicにcommitできます。

`$expr`による同一record内のfield比較を提供します。
比較operandには二つの数値operandを持つ`$add`と`$subtract`を含められます。
整数結果は収まる場合に整数を維持し、混在型または小数の結果は有限なJSON数値でなければなりません。
欠損または非数値のfield operandは比較不一致になり、整数overflowと有限でない結果は拒否します。

predicateは`$eq`、`$ne`、`$gt`、`$gte`、`$lt`、`$lte`、`$in`、`$nin`、`$and`、`$or`、`$not`、限定された`$expr`です。

HTTP境界の詳細は[HTTP method semantics](docs/ja/http-semantics.md)を参照してください。

`HEAD /records`と`HEAD /records/{id}`はGETと同じstatus/headerを返し、response bodyを転送しません。

成功する`GET /records`と`GET /records/{id}`は現在のtable representationを示すstrongな`ETag`を返します。
GETとHEADは`If-None-Match`にも対応し、一致すれば`304 Not Modified`を返します。
`POST`、`PUT`、`PATCH`、`DELETE`、`POST /transaction`には任意の`If-Match`を付けられます。
single-tableの状態変更には`If-None-Match`も付けられ、一致すれば`412 Precondition Failed`を返して変更しません。
`If-Match`にcurrentなstrong tagまたは既存resourceに対する`*`以外を指定すると`412 Precondition Failed`となり、tableは変更されません。
WAL-backed mutationは`X-Txbase-Transaction-Id`を返し、single-tableの`POST /transaction`では同じ値をJSONの`transaction_id`にも含めます。
catalog serverの`POST /transaction`は新しいcatalog representation `ETag`も返します。
`If-Match`と`If-None-Match`を任意で指定でき、条件に失敗すると`412 Precondition Failed`となり、DBFやsidecarを変更しません。

### Mutation

`POST`はrecordを作成します。

`PUT`はrecord全体を置き換えます。

`PATCH`はrecordを部分更新し、`$set`、`$unset`、`$inc`を使えます。

`DELETE`はDBFのdeletion markerを設定します。

複数のrecord mutationは`POST /transaction`で一つのDBFに対してまとめてcommitできます。
全operationをprivate copyに適用してから、一回のsnapshot/WAL boundaryで保存します。
operationが失敗した場合はcopyを破棄し、元のDBFを変更しません。
commit IDは`.txbase.state` sidecarに保存され、再起動またはWAL復旧後も継続します。cross-table atomicityはcatalog journalが担当し、catalog journalのcommit IDは`.txbase.catalog.state`に保存されます。historical row versionとMVCC visibilityは未実装です。

path-loaded mutationは、可能なら`TXDP` byte-range deltaを使います。

それ以外の場合は`TXDB`または`TXDM` snapshotを使います。

WALをsyncしてからDBFまたはmemo sidecarを置き換えます。

起動時には未完了のmutationを復旧します。

既存index sidecarがある場合、その更新先も同じWALに記録し、crash後にWALを消す前に再適用します。

### Inspectとmaintenance

schemaとverifyはDBFを読み取り、schemaとrecord boundaryを確認します。

DBF headerのlanguage-driver byteが信頼できない場合は、read、schema、verify、pack、recall、serverで`--encoding NAME`を指定できます。

対応するaliasは`windows-31j`/`cp932`、strictな`shift_jis`/`shift-jis`/`sjis`、`gbk`/`cp936`、`euc-kr`/`cp949`、`big5`/`cp950`、`euc-jp`、`gb18030`、`iso-2022-jp`/`iso2022-jp`です。

invocation単位のoverrideは`*.txschema.json`より優先され、保存されません。

effectiveな値はschema outputの`encoding_override`または`encoding_metadata.effective`で確認できます。
`encoding_metadata`はdeclared codecと、language-driver、explicit override、fallbackのsourceも返します。

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
txbase xbf import path/to/users.dbf path/to/users.xbf
txbase xbf export path/to/users.xbf path/to/users.dbf
txbase xbf export path/to/users.xbf path/to/users.dbf --schema
txbase xbf report path/to/users.xbf
```

`pack`はlogical delete済みrecordを物理的に除去し、残ったrecord numberを詰め直します。

`recall`はphysical record numberを指定してlogical deleteを取り消します。

`xbf import`はDBF tableをbounded XBF snapshotへ変換します。
`xbf export`は表現可能なXBF subsetだけをDBFへ変換し、unsupported typeやvalueはerrorにします。
`XbfTable::dbf_export_report`はfileを書かずに表現可能性を調べ、field/record単位の問題とschema
sidecarが必要かどうかを返します。
libraryの`XbfTable::to_dbf_with_schema`は、表現可能な`primary`、`unique`、`not_null`のsidecar JSONを返します。
`XbfTable::save_dbf_with_schema`とCLIの`xbf export --schema`は、DBF、`.txschema.json`、memo sidecar、`.txbase.state`のcommit IDを
recoverableな`TXSE` export boundaryでjournal化します。途中で停止した場合は次のDBF readで復旧し、別writerが変更したtargetは上書きしません。
外部のlegacy readerに対する複数fileの物理的atomic snapshotまでは保証しません。

`catalog`はdirectory直下のDBF tableを発見し、各tableのschemaを表示します。

`verify-catalog`は発見したすべてのtableをverifyします。

`index build`はscalar keyのexternal sidecarを作成します。

`index verify`はDBFまたはmemo sidecarが変更されたindexをstaleとして拒否します。

通常のDBF保存は既存sidecarの更新先をWAL経由で適用し、staleまたはinvalidなsidecarは拒否します。

`index rebuild`は明示的な修復手段です。

path-aware plannerは、複数のsingle-field indexが有効なdirect equality filterであれば候補recordをintersectionできます。

catalog joinは`txbase::query::join::parse`と`execute`から使います。
複数joinは実装済みです。
直接および多段の単一キー等値結合ステージは、64候補pairの境界を超えると有界なhashコストとindex probeコストを比較し、推定値が低い場合だけ鮮度検証済みの単一フィールドindexを使います。
直接および多段ステージは、等値条件のフィールド順が複合indexのフィールド順と完全に一致する場合、その鮮度検証済み複合indexも使います。
両入力に互換性のある鮮度検証済みordered indexがある大きな直接joinは、推定値が低い場合に文書化した出力順を戻しながら有界なmerge strategyを使います。
それ以外の等値結合は候補pairが64以下ならnested-loop、それより大きければhash strategyを選びます。
full index-awareまたはcost-based merge join planning、full cost-based planner、runtime固有のasync stream trait、historical row version、MVCC visibilityは未実装です。

backupとrestoreは、DBFと同じstemの`.dbt`または`.fpt`、`.txschema.json`、`.txbase.state`、有効な`.txidx` sidecarもコピーします。
sourceのindexがstaleまたは壊れている場合は拒否し、sourceにindexがなければdestinationの古いindexを削除します。

DBF codecはVisual FoxProのCJK driver IDであるWindows-31J/CP932、GBK/CP936、EUC-KR/CP949、Big5/CP950に対応します。
`euc-jp`、`gb18030`、`iso-2022-jp`はlanguage-driver IDを追加せず、explicit overrideとして使えます。
malformed readはU+FFFDにし、unmappableまたはbyte width超過のwriteは拒否します。
strictな`Shift_JIS` overrideはASCII、半角カナ、JIS X 0208を受け付け、CP932拡張はreadでU+FFFD、writeで拒否します。

optionalな`users.txschema.json` sidecarは、legacy DBF byteを変更せずに一つのfieldへ`primary`、`unique`、`not_null`を設定し、boundedなcomposite `primary`と`unique`も定義できます。
同じsidecarで対応済みのCJK codecを明示的に選択できます。
load済みのactive recordとinsert、replace、patch、recallをconstraintで検証します。
省略されたinsert fieldへのscalar `default`、table-level `checks`、catalog-scopedな`references`も実装済みです。
`references`はcatalog serverのnamed-table mutationとcross-table transactionで検証し、single-table pathでは解決しません。

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
src/catalog/journal.rs catalog lock、journal、crash recovery
src/catalog/transaction.rs named-table transactionの準備とcommit
src/index.rs        scalar keyのexternal index sidecar lifecycle
src/query.rs        JSON queryの実行とvalidation
src/query_path.rs   dotted pathとprojectionのhelper
src/server.rs       HTTP routing、QUERY validation、共通HTTP response
src/server/records.rs DBF recordのGET/POST/PUT/PATCH/DELETEとmutation persistence
src/server/etag.rs  HTTP representation validatorとconditional request
src/server/catalog.rs catalog schema、named-table HTTP surface、bounded join
src/server/catalog_transaction.rs catalog cross-table transaction HTTP surface
src/server/explain.rs query plan explanation HTTP surface
src/transaction.rs  fileまたはmemory WALとsnapshot transaction
src/xbf/            bounded XBF v1 codec、DBF変換、永続化、WAL、test
tests/fixtures/     外部formatのfixture
tests/corpus/       malformed inputのcorpus
docs/               仕様調査と設計判断
```

fileは責務と保守性が明確になる粒度で分割します。

crate分割はbuildまたはownershipの境界が必要になるまで行いません。

### 詳細文書

- [日本語ドキュメント一覧](docs/ja/README.md)
- [DBFとdBASE互換性](docs/ja/dbf-compatibility.md)
- [Multi-table catalog](docs/ja/catalog.md)
- [Secondary-index sidecar](docs/ja/indexes.md)
- [Query model](docs/ja/query-model.md)
- [Query planning](docs/ja/query-planning.md)
- [Mutation model](docs/ja/mutation-model.md)
- [Firebase data model](docs/ja/firebase-model.md)
- [SQLite testingとquality](docs/ja/testing-quality.md)
- [Quality contract matrix](docs/ja/quality-matrix.md)
- [HTTP semanticsとQUERY](docs/ja/http-semantics.md)
- [Schema metadataとlocal constraint](docs/ja/schema-metadata.md)
- [XBF v1 format draft](docs/ja/xbf.md)
- [Roadmapと明示的なnon-goal](docs/ja/roadmap.md)
- [Research index](docs/ja/research.md)

## Todo

`xbf report`はファイルを書き込まず、XBFの型と値がDBFへ変換可能かとschema sidecarの要否を確認します。

当面はDBF、memo、WAL、query、HTTPのcontractをfixtureとfailure testで固めます。

secondary index sidecarの自動更新と単純なequality、uniform selectivity estimateによるequality intersection、histogram estimateによるsingle-field range planner、single-field ordered planner、multi-key sortのordered-prefix planner、fieldごとのdirectionを持つcompound indexによるmulti-key sort plannerを実装済みです。

equality、range、ordered candidateが同時に有効な場合は、active record数または正確なcandidate数、サイドカーの走査、残りのsort作業に基づく有界な整数コストでtable scanとindex経路を比較します。

これはI/O、memory、cache、collation、compound rangeを含む完全なcost modelではありません。
一つのfieldに対する`primary`、`unique`、`not_null`のschema metadataも実装済みです。
I/Oを考慮したfull cost model、collation-aware planning、runtime固有のasync stream trait、追加のaggregation stage、cross-table constraint、追加のCJK encodingはroadmapで検討します。
XBFのwire contractは[XBF v1 format draft](docs/ja/xbf.md)に記載しています。draftのcodec、DBFからXBFへの変換、限定されたXBFからDBFへのexport、schema sidecar付きjournal export、durable snapshot path、generation付きfull-snapshot WAL recoveryを提供します。strictな複数file reader atomicityとobject-storage commitは未対応です。

## License

MITです。
