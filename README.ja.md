# txBASE

txBASEは、dBASEのDBF形式を中心に据えた小規模なRust製データベース・プロトタイプです。

<table>
<tr>
<td><a href="README.md">English</a></td>
<td><a href="README.ja.md">日本語</a></td>
</tr>
</table>

## txBASEが提供するもの

txBASEは、元のDBF表現を保ったまま、dBASEとVisual FoxProの一部のフィールドを読み書きします。

- 単一テーブルまたはディレクトリ内の複数テーブルに対するJSONとHTTPのアクセス。
- 更新を復旧可能にするファイルベースWALとmemoサイドカー。
- 有界なクエリ、集約、ローカルなカタログ結合、外部スカラー／複合インデックス。
- XBF v1スナップショットと、DBFへ変換できる値を報告するエクスポート。
- 世代CASと復旧を備え、メモリまたは永続ファイルシステムストアを使える決定的なXBFオブジェクトストレージマニフェスト境界。
- 既存のDBFバイト列を変更しない明示的なCJK codec選択。

互換性と動作の詳細は、[ドキュメント一覧](docs/ja/README.md)にまとめています。

## 使い方

### 最短の手順

DBFの有効なレコードを読み取ります。

```bash
cargo run -- read path/to/users.dbf
```

単一テーブル用のHTTPサーバーを起動します。

```bash
cargo run -- serve path/to/users.dbf
```

名前付きテーブルと有界な結合を使うカタログサーバーを起動します。

```bash
cargo run -- serve-catalog path/to/database
```

既定のリスナーは`127.0.0.1:8080`です。
`--bind ADDRESS`で別のアドレスを指定できます。

### 読み取りとクエリ

```bash
curl -s http://127.0.0.1:8080/records | jq
curl -s http://127.0.0.1:8080/records/1 | jq

curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"sort":{"NAME":1},"limit":10}' \
  http://127.0.0.1:8080/records
```

`GET`と`HEAD`では、DBFの物理レコード番号を1から数えて指定します。
`QUERY /records`は、有界な`filter`、`sort`、`projection`、`collation`、`skip`、`limit`、`page_size`、`cursor`、`aggregate`を受け付けます。

`page_size`を指定すると、不透明なcursorを返します。
物理cursorはレコード順に進み、ソート付きcursorは指定されたsortと物理レコード番号のタイブレーカーを使います。
新しく発行するcursorは現在のテーブル表現に束縛されますが、互換性のため、旧形式の束縛されていないcursorも受け付けます。

`QUERY /records/stream`は、filter、projection、skip、limitの結果をchunkedな`application/x-ndjson`で返します。
`QUERY /explain`は、選択されたテーブルスキャンまたはインデックス計画を返します。
ライブラリには、対応するborrowed、snapshot、boundedのストリームiteratorもあります。

カタログサーバーは、`GET /catalog`、名前付きテーブルの読み書き、`QUERY /{table}/records`、`QUERY /{table}/explain`、`QUERY /{table}/records/stream`、有界なローカル結合`QUERY /join`を提供します。
`POST /transaction`は、カタログjournalを通じて名前付きテーブルの更新をcommitします。

`QUERY /join`は、有界な`inner`、`left`、`right`、`full`、`semi`、`anti`、`cross`結合をサポートします。
大きな直接等値結合では、freshで互換性のあるordered indexを使ったmerge実行を選べます。
そのmerge経路を使えない場合、またはコストが高い場合は、有界なhash経路またはindex probe経路にフォールバックします。

正確な境界は、[クエリモデル](docs/ja/query-model.md)、[集約モデル](docs/ja/aggregation.md)、[結合モデル](docs/ja/joins.md)、[クエリ計画](docs/ja/query-planning.md)を参照してください。

### 更新

```bash
curl -i -X POST \
  -H 'Content-Type: application/json' \
  -d '{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}' \
  http://127.0.0.1:8080/records

curl -i -X PATCH \
  -H 'Content-Type: application/json' \
  -d '{"NAME":"Caroline"}' \
  http://127.0.0.1:8080/records/3

curl -i -X PATCH \
  -H 'Content-Type: application/merge-patch+json' \
  -d '{"NAME":"Alicia","AGE":null}' \
  http://127.0.0.1:8080/records/3

curl -i -X PATCH \
  -H 'Content-Type: application/json-patch+json' \
  -d '[{"op":"replace","path":"/NAME","value":"Alicia"}]' \
  http://127.0.0.1:8080/records/3

curl -i -X DELETE http://127.0.0.1:8080/records/3
```

`POST`、`PUT`、`PATCH`、`DELETE`は、それぞれレコードの作成、置換、更新、論理削除をします。
`PATCH`は、ローカル形式の`application/json`更新文書、`application/merge-patch+json`のオブジェクトパッチ、有界な`application/json-patch+json`操作配列を受け付けます。
Merge Patchはオブジェクトを再帰的にマージし、`null`をフィールド削除として扱い、配列またはスカラー値で現在値を置き換えます。
JSON PatchはRFC 6901のJSON Pointerパスを使うRFC 6902の`add`、`remove`、`replace`、`test`、`move`、`copy`をサポートします。
ローカル形式のJSON文書では、型付きの`$set`、`$unset`、`$inc`演算子も使えます。

単一テーブルの`POST /transaction`は、複数の操作をprivate copyへ適用してから、1つのsnapshot/WAL境界でcommitします。
単一テーブルの更新は、`mvcc` CLIを通してテーブル単位の過去スナップショットを保持します。
カタログの`POST /transaction`も、検出したすべてのテーブルの完全なイメージによる過去スナップショットを保持します。
`Catalog::from_path_at`と`mvcc catalog`は、一貫したカタログcommitを読み取ります。
カタログHTTPサーバーは、現在のイメージだけを提供します。

成功した更新は`X-Txbase-Transaction-Id`を返し、commit IDをstateサイドカーへ保存します。
strongな`ETag`と`If-Match`、`If-None-Match`によって、古い更新を部分変更なしで拒否できます。
WALと復旧の契約は、[更新モデル](docs/ja/mutation-model.md)に記載しています。
過去スナップショットの可視性は、[MVCC文書](docs/ja/mvcc.md)に記載しています。

### 検査と保守

読み取り専用のschema、catalog、index、XBF、WAL検査。

```bash
txbase schema path/to/users.dbf
txbase verify path/to/users.dbf
txbase catalog path/to/database
txbase verify-catalog path/to/database
txbase mvcc list path/to/users.dbf
txbase mvcc read path/to/users.dbf 1
txbase mvcc gc path/to/users.dbf --keep 5
txbase mvcc catalog list path/to/database
txbase mvcc catalog read path/to/database 1
txbase mvcc catalog gc path/to/database --keep 5
txbase index verify path/to/users.dbf
txbase xbf report path/to/users.xbf
txbase wal inspect path/to/users.txbase.wal
```

`wal inspect`はWALを作成せず、切り詰めずに読み取ります。
ファイルサイズ、有効なバイト境界、完全なレコードのLSNとペイロード長、最後のレコードが切断されているかどうかを表示します。

`mvcc gc`と`mvcc catalog gc`は、完全イメージのうち新しいものを指定した正の件数だけ保持し、同期済み一時ファイルを通してMVCC履歴サイドカーだけを書き替えます。

現在のDBFとカタログ状態は変更せず、GCで削除したスナップショットIDは読み取れなくなります。

インデックスのライフサイクルと保守。

```bash
txbase index build path/to/users.dbf NAME AGE
txbase index build-compound path/to/users.dbf by_name_age NAME AGE
txbase index rebuild path/to/users.dbf
txbase pack path/to/users.dbf
txbase recall path/to/users.dbf 2
```

`index verify`は古いサイドカーを拒否し、`index rebuild`が明示的な修復手段になります。
`pack`は論理削除したレコードを除去し、`recall`は物理レコード番号で一件の論理削除を取り消します。

XBF変換とサイドカーを含むファイル転送。

```bash
txbase xbf import path/to/users.dbf path/to/users.xbf
txbase xbf export path/to/users.xbf path/to/users.dbf
txbase xbf export path/to/users.xbf path/to/users.dbf --schema
txbase backup path/to/users.dbf backups/users.dbf
txbase restore backups/users.dbf path/to/users.dbf
```

`xbf report`はファイルを書き込まずに変換可能性を検査します。
`xbf export --schema`はDBF、schema、memo、stateのサイドカーを復旧可能な`TXSE`境界でjournal化しますが、外部のlegacy readerに対する物理的な一括スナップショットまでは保証しません。
`backup`と`restore`は、対応するmemo、schema、state、有効なindexのサイドカーを検証してコピーします。

### ファイルとサイドカーの境界

`users.dbf`に対して、txBASEは兄弟ファイルの`.dbt`または`.fpt`のmemoデータ、`users.txschema.json`の制約とencoding metadata、`users.txbase.wal`、有効な`users.txidx`のスカラーまたは複合インデックスのサイドカーを検出することがあります。

DBFのlanguage-driver byteがない、または信頼できない場合、read、schema、verify、pack、recall、serverの各コマンドで`--encoding NAME`を指定できます。
対応するaliasは、`windows-31j`/`cp932`、`gbk`/`cp936`、`euc-kr`/`cp949`、`big5`/`cp950`、strictな`shift_jis`/`shift-jis`/`sjis`、`euc-jp`、`gb18030`、`iso-2022-jp`/`iso2022-jp`です。

不正な入力の読み取り結果はU+FFFDになります。
変換できない値や幅を超える値の書き込みは拒否します。
対応形式とサイドカーの契約は、[DBF互換性](docs/ja/dbf-compatibility.md)、[schema metadata](docs/ja/schema-metadata.md)、[XBF v1](docs/ja/xbf.md)に記載しています。

## インストール

CIはstable Rustを使い、Ubuntu、macOS、Windowsを対象にします。

[rustup](https://rustup.rs/)でRustをインストールしてから、バイナリをビルドします。

```bash
cargo build --release
```

生成されたバイナリは`target/release/txbase`です。

## 開発

### 品質ゲート

CIはUbuntuのdocsジョブでドキュメントを検査し、Ubuntu、macOS、WindowsのRustジョブで次のチェックを実行します。

```bash
bun install --frozen-lockfile
bun run lint:docs
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

### リポジトリ構成

```text
src/dbf/            DBF parser、codec、memoサイドカー、保守、更新、WAL、テスト
src/catalog.rs      直下のDBF検出、テーブル検索、catalog検証
src/catalog/        catalog lock、journal、復旧、名前付きテーブルのtransaction
src/catalog/mvcc.rs catalog全体のcommit単位の過去スナップショット
src/index.rs        外部scalar／compound keyのindexサイドカー
src/query.rs        JSON queryの実行とfilter評価
src/query/          planner、ordering、validation、有界join、queryテスト
src/server.rs       HTTP routing、QUERY validation、共通response
src/server/         record、catalog、ETag、explain、transactionのroute
src/transaction/    fileまたはmemoryのWALとsnapshot transaction
src/xbf/            bounded XBF v1 codec、DBF変換、永続化、WAL、テスト
tests/fixtures/     外部形式のfixture
tests/corpus/       壊れたDBF、memo、WAL、JSONの入力
docs/               トピック別の仕様と設計メモ
```

ファイルは、責務、失敗時の挙動、fixture、変更頻度のいずれかが異なる場合に分割します。
crateの分割は、実際のbuildまたはownershipの境界が必要になるまで行いません。

### ドキュメント

- [ドキュメント一覧](docs/ja/README.md)
- [英語ドキュメント](docs/README.md)
- [DBFとdBASEの互換性](docs/ja/dbf-compatibility.md)
- [複数テーブルのcatalog](docs/ja/catalog.md)
- [セカンダリインデックスのサイドカー](docs/ja/indexes.md)
- [クエリモデル](docs/ja/query-model.md)
- [集約モデル](docs/ja/aggregation.md)
- [結合モデル](docs/ja/joins.md)
- [クエリ計画と外部語彙](docs/ja/query-planning.md)
- [更新モデル](docs/ja/mutation-model.md)
- [Firebaseのデータモデルと同期から得た知見](docs/ja/firebase-model.md)
- [SQLiteのテストと品質モデル](docs/ja/testing-quality.md)
- [品質契約マトリクス](docs/ja/quality-matrix.md)
- [HTTPメソッドの意味とQUERY](docs/ja/http-semantics.md)
- [schema metadataとローカル制約](docs/ja/schema-metadata.md)
- [XBF v1フォーマット草案](docs/ja/xbf.md)
- [MVCCと過去スナップショット](docs/ja/mvcc.md)
- [ロードマップと明示的な非目標](docs/ja/roadmap.md)
- [調査インデックスと出典ポリシー](docs/ja/research.md)
- [エッジストレージとオブジェクトストレージのコミット](docs/ja/edge-storage.md)
- [WASMとワーカーのホスト境界](docs/ja/wasm.md)
- [分散化の進化](docs/ja/distributed-evolution.md)

## 現在の境界とロードマップ

現在の実装は、無制限のデータベースサーバーよりも、有界で復旧可能なローカル処理を優先します。

次の領域は引き続き将来の作業です。

- runtime固有のasync stream trait。
- 完全なcost-based index／join planner。
- より広い集約。
- 行単位のMVCC履歴と保持期間。
- locale-awareなCJK collation。
- 追加のupstream CJK fixture。
- XBF exportにおける厳密な複数ファイルreader atomicity。
- クラウドオブジェクトストレージアダプターと保持方針。
- 分散replication。

受け入れ条件は[docs/ja/roadmap.md](docs/ja/roadmap.md)に、出典とfixtureの方針は[docs/ja/research.md](docs/ja/research.md)に記載しています。

## ライセンス

MITです。[LICENSE](LICENSE)を参照してください。
