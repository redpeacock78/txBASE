# 複数テーブルカタログ

カタログ境界は、1つのデータベースディレクトリを、その直下に保存されたDBFテーブルへ対応付けます。

これはロードマップで最初に定めた複数テーブルのスライスです。

カタログジャーナルのcommitは、検出したすべてのテーブルの一貫した過去イメージも保持します。

リレーション、テーブル間インデックス定義、別のストレージ形式は追加しません。

## ファイルシステム契約

`Catalog::from_path`は存在するディレクトリを受け取ります。

拡張子が大文字小文字を問わず`.dbf`である通常ファイルを、1つのテーブルとして扱います。

テーブル名はファイル名の拡張子を除いた部分です。

たとえば次の構成です。

```text
database/
├── comments.dbf
├── posts.dbf
├── posts.dbt
└── users.dbf
```

カタログは`comments`、`posts`、`users`を公開します。

memoサイドカー、WALファイル、ロックファイル、無関係なファイルはカタログ項目になりません。

検出するのは直下のファイルだけです。

公開APIではテーブル名を完全一致で照合します。

カタログはマニフェストを書き込まず、ファイル名を変更せず、サイドカーの所有権も主張しません。

`.txbase.catalog.mvcc`履歴は内部用のバージョン付きスナップショットサイドカーであり、テーブル検出用マニフェストではありません。

これによりDBFファイルを既存のxBaseツールで読み取れ、テーブル単位の復旧は既存のDBF経路に任せられます。

## Rust API

```rust
use txbase::catalog::Catalog;

let catalog = Catalog::from_path("database")?;
let users = catalog.open_table("users")?;
let names = catalog.table_names();
catalog.verify()?;

let versions = Catalog::mvcc_versions("database")?;
let historical = Catalog::from_path_at("database", 1)?;
let old_users = historical.open_table("users")?;
let read = catalog.begin_read()?;
let stable_users = read.open_table("users")?;
let stable_rows = read.execute_join(&join_request)?;
let mut transaction = catalog.begin_serializable()?;
transaction.apply(&operation)?;
let transaction_id = transaction.commit()?;
```

`open_table`は既存の復旧およびmemoサイドカー経路を通して1つのDBFをロードします。

`table_path`はテーブルをロードせずにパスを返します。

`schema_json`は検出した全テーブルをロードし、名前、ファイル名、DBFスキーマを返します。

`transaction_id`は、そのカタログが表すカタログcommit IDを返します。

現在のカタログでは、存在すれば最後に永続化したカタログジャーナルのcommit IDです。

過去のカタログでは、開くときに指定したスナップショットIDです。

`mvcc_versions`はイメージを保持しているカタログcommitを列挙し、`from_path_at`は読み取り専用のイメージを開きます。

その値から開いたすべてのテーブルは、同じカタログcommitに属します。

過去テーブルは保持したDBF、memo、スキーマのイメージを使い、現在のインデックスサイドカーを再利用しません。

`Catalog::begin_read`は、検出したすべての現在テーブルを1つのプロセス内読み取り専用イメージとして取得します。

最初に保留中のカタログ状態とテーブル状態を復旧します。

次にカタログ読み取りロックと、テーブル名順に取得したすべてのテーブル読み取りロックの下で、DBF、memo、スキーマのバイト列を読み込みます。

取得後にロックを解放するため、読み取りオブジェクトを保持しても後続の更新をブロックしません。

`CatalogReadTransaction::transaction_id`は、取得時に観測したカタログcommit IDを返します。

カタログジャーナルのcommitがまだ存在しない場合は`None`です。

`open_table`は独立した読み取り専用テーブルコピーを返し、`execute_join`は連鎖ステージを含む既存の有界結合契約を取得したテーブルに対して実行します。

取得した結合はライブのインデックスサイドカーを使いません。

結果数とステージ数の上限は維持し、スキャンまたはハッシュの実行経路を使います。

`Catalog::begin_serializable`は、検出したすべてのテーブルを対象にする、任意選択の粗粒度serializableトランザクションを開きます。

beginからcommit、rollback、またはdropまで、カタログwrite lockとすべてのテーブル排他ロックを保持します。

`CatalogTransaction::apply`は非公開のテーブルコピーだけを変更し、`commit`はテーブル間制約を検証して1つのカタログjournalトランザクションを公開します。

`open_table`は非公開イメージの読み取り専用コピーを返します。

この境界は検出済みテーブル集合をカタログ全体で直列化します。
トランザクション開始時のDBF名とファイル名の集合を記録します。
commit時の集合が異なる場合は、テーブルを暗黙に除外または追加せず、トランザクションを拒否します。
述語単位の並行性と分散調整は提供しません。

このメモリ内トランザクションは永続的な履歴のピン留めではなく、MVCCの保持期間も延長しません。

プロセス終了後に保持されたcommitを再び開く場合は`Catalog::from_path_at`を使います。

IDが進むのは複数テーブルの`POST /transaction`ジャーナル境界だけです。

独立した名前付きテーブルの更新はテーブルごとのDBFトランザクションIDを保ち、新しいカタログイメージを作りません。

`verify`は検出した全テーブルと存在するインデックスサイドカーをロードして検証し、テーブルまたはサイドカーが失敗したときにテーブル名を報告します。

有界ローカル結合の境界はカタログ検出とは別です。

検証済みの結合文書と`Catalog`を`txbase::query::join::execute`に渡すと、永続的な関係マニフェストを追加せずに1つ以上の名前付きテーブルを読み取れます。

任意のカタログサーバーはこの境界をHTTPで公開します。

```bash
txbase serve-catalog path/to/database --bind 127.0.0.1:8080
```

`GET`または`HEAD /catalog`は検出したテーブルスキーマと、カタログ表現を示す強い`ETag`を返します。

強いタグまたは弱いタグが一致する`If-None-Match`、あるいは`*`を指定すると、本文なしの`304 Not Modified`を返します。

`GET`または`HEAD /{table}/records`と`/{table}/records/{id}`は、現在の表現`ETag`を含む単一テーブルのレコード応答意味論を再利用します。

`QUERY /{table}/records`は単一テーブル経路と同じJSONクエリ文書を受け付けます。

`QUERY /{table}/records/stream`は`filter`、`projection`、`skip`、`limit`に対応する有界`application/x-ndjson`ストリームを公開します。

`QUERY /{table}/explain`は単一テーブルの`QUERY /explain`と同じ選択済み計画を返します。

`QUERY /join`は`query::join::parse`と同じJSON結合文書を受け付け、JSON配列を返し、結合ステージごとに100,000行の上限を維持します。

`POST /{table}/records`と`PUT`、`PATCH`、`DELETE /{table}/records/{id}`は、単一テーブルの更新、WAL、ETag、検証、制約の動作を再利用します。

各リクエストは名前付きDBFだけをコミットします。

`POST /transaction`は名前付きテーブルの更新パスを受け取り、カタログジャーナルの下で影響を受ける全DBFと変更されたインデックスサイドカーをコミットします。

同時に、検出したすべてのテーブルのイメージをカタログMVCC履歴へ記録します。

履歴イメージを含む未完了の準備は、次のカタログ読み取り時にロールバックします。

成功したカタログジャーナルコミットは永続的なカタログトランザクションIDを進め、JSON本文と`X-Txbase-Transaction-Id`ヘッダーで返します。

応答は新しいカタログ表現`ETag`も返します。

任意の`If-Match`と`If-None-Match`はカタログwrite lockの内側で評価します。

`If-Match`は現在の強いタグまたは`*`を要求し、弱いタグまたは一致しない値には`412 Precondition Failed`を返します。

強いタグまたは弱いタグが一致する`If-None-Match`、あるいは`*`にも同じ応答を返し、DBFやサイドカーを変更しません。

トランザクションIDは、カタログHTTPサーバーで1リクエストの過去読み取りを選ぶ値にもなります。

長寿命トランザクションを作らず、行単位の可視性も提供しません。

成功した`POST /transaction`は、`.txbase.catalog.cdc`へ`TXCC`イベントを1つ公開します。
イベントは、そのカタログcommitで変更したすべてのテーブルについて、物理レコードの状態差分を持ちます。
カタログジャーナルは、DBF、インデックス、MVCC、トランザクション状態の対象と一緒にCDCサイドカーを適用またはロールバックします。

カタログの検出はサーバー起動時に一度行います。

各リクエストは既存の復旧経路を通して名前付きテーブルをロードします。

名前付きテーブルの更新は、テーブルを追加または削除しません。

過去のカタログイメージは、カタログHTTPサーバーからも公開します。

`GET`または`HEAD /catalog`、`GET`または`HEAD /{table}/records[/{id}]`、`QUERY /{table}/records`、
`QUERY /{table}/records/stream`、`QUERY /{table}/explain`、`QUERY /join`は、
`?at=<正のcommit済みカタログトランザクションID>`を受け付けます。

1つのリクエストは、そのカタログcommitに含まれる全テーブルの1つのイメージを読み取ります。

過去のクエリと結合は現在のインデックスサイドカーを再利用せず、過去の説明応答はテーブルスキャンを返します。

`at`は読み取り専用です。

このパラメーターを付けたカタログ更新には`405`を返します。

0、不正、重複した`at`パラメーターには`400`を返します。

保持されているcommit済みカタログスナップショットでないIDには`422`を返します。

不正なDBFがあっても、検出はファイルの識別だけを行うため、ディレクトリ検出自体は失敗しません。

不正なテーブルは`open_table`、`schema_json`、`verify`が報告します。

## CLI

カタログと全テーブルのスキーマを表示します。

```bash
txbase catalog path/to/database
```

検出した全テーブルを検証します。

```bash
txbase verify-catalog path/to/database
```

保持しているカタログcommitを列挙し、一貫した過去イメージを読み取ります。

```bash
txbase mvcc catalog list path/to/database
txbase mvcc catalog read path/to/database 1
txbase mvcc catalog gc path/to/database --keep 5
txbase cdc catalog path/to/database
txbase cdc catalog path/to/database --after 10
```

カタログ出力の形は次のとおりです。

```json
{
  "format": "txbase-catalog",
  "transaction_id": null,
  "tables": [
    {
      "name": "users",
      "file": "users.dbf",
      "schema": {
        "format": "dbf"
      }
    }
  ]
}
```

完全なネストスキーマには、DBFヘッダー、レコード数、memoサイドカーの検出結果、フィールド記述子が含まれます。

## 境界

カタログは現在、検出、検索、スキーマの内省、検証、有界ローカル結合が使う入力境界、名前付きテーブルを独立して読み書きする任意のHTTP境界、読み取り専用のカタログ全体の過去スナップショットを提供します。

`CatalogReadTransaction`は、取得後に更新をブロックしない、テーブル横断の安定した読み取りイメージも提供します。

名前付きレコードの更新を横断するアトミックなトランザクション境界も提供します。

`Catalog::begin_serializable`は、カタログwrite lockと検出したすべてのテーブルロックをトランザクション中は保持し、非公開テーブルコピーを同じカタログjournalでcommitする、任意選択の粗粒度serializable Rustトランザクションも提供します。

この複数テーブルcommitは`Catalog::cdc_events`と`txbase cdc catalog DIRECTORY`で公開します。
カタログCDCストリームは明示的な複数テーブルトランザクションの境界に限られ、独立した名前付きテーブル経路はテーブル単位のCDCイベントを保持します。

カタログジャーナルは単調増加するコミットIDを`.txbase.catalog.state`に永続化します。

成功した複数テーブルcommitは、検出したすべてのテーブルの完全なイメージを`.txbase.catalog.mvcc`へ同じジャーナルを通して保存します。

準備済みジャーナルでは状態と履歴をロールバックし、コミット済みジャーナルでは両方を再適用して復旧します。

履歴が提供するのはカタログイメージに対するcommit単位の可視性です。

`Catalog::gc_mvcc`とカタログGCコマンドは、完全イメージのうち新しいものを指定した正の件数だけ保持し、同期済み一時ファイルを通して履歴サイドカーだけを置き換えます。

GCで削除したIDは読み取れず、次のカタログcommitは保持されたIDの後ろに追加されます。

過去のカタログスナップショットに対するテーブル単位の行履歴は公開せず、その履歴は直接のテーブルMVCCが別に公開します。

独立した行保持、分散スナップショット、述語単位のserializable競合検出、行単位の書き込み競合マージは提供しません。

`Catalog::begin_serializable`は、検出済みテーブル集合に対する粗粒度の直列実行を提供します。

commit時に検出済みDBFの名前とファイル名の集合が変わっていないことを検証します。

述語単位のロックと分散serializable調整は提供しません。

フィールドサイドカーが`references: "TABLE.FIELD"`を宣言すると、カタログの名前付き更新とカタログトランザクションは、非nullの子値が参照テーブルのアクティブ行に存在するか検証します。

nullは許可します。

任意の`on_delete`と`on_update`は既定で`restrict`です。

`cascade`は一致する親の削除またはキー更新を子へ伝播し、`set_null`はローカルキーをnullにします。

`set_null`にはnullを許す非主キーのローカルフィールドが必要です。

スキーマサイドカーは、同じ長さの子側と親側のフィールド列を持つ`constraints.foreign_keys`も宣言できます。

カタログはタプル全体を比較し、子側の値のいずれかがnullなら検査を省略し、親の更新や論理削除に同じ動作を適用します。

連鎖は1つのカタログトランザクションとジャーナルcommitの内部で再帰的に適用します。

制約違反または収束しない連鎖は、どのテーブルも公開する前に拒否します。

直接の単一テーブル経路はテーブル間規則を解決できません。

カタログロックはカタログの読み書きを直列化し、テーブル単位のロックは直接のDBF永続化を保護します。

フィールド名からリレーションを推測することはありません。

ローカル結合は`inner`、`left`、`right`、`full`、`semi`、`anti`の等値結合と、有界な`cross`結合をサポートし、結果数に上限を持ちます。

直接の等値結合では、フィルター前の基数、出力マテリアライズ、DBF論理ページの決定的な入力を戦略選択に含めます。
多段の等値ステージもこれらの入力を伝播し、条件を満たす非`full`ステージでは[結合モデル](joins.md)に記載したordered merge経路を使用できます。
カタログは、ファイルシステムとキャッシュを考慮したmerge計画、ホスト固有の`AsyncQueryStream`スケジューリング、行単位のMVCCバージョン、分散可視性を提供しません。

## 一次資料と適用範囲

カタログはtxBASEが所有するディレクトリとトランザクションの契約であり、外部カタログ標準の実装ではありません。
DBFとサイドカーの規則は[DBF互換性](dbf-compatibility.md)に、ローカル結合の意味は[結合モデル](joins.md)に、フィールド制約は[スキーマメタデータ](schema-metadata.md)に委ねます。

txBASEが分散カタログまたは外部との互換性目標を選ぶまでは、この文書に外部カタログ資料は置きません。
