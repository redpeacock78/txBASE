# 複数テーブルカタログ

カタログ境界は、一つのデータベースディレクトリを、その直下に保存された DBF テーブルへ対応付けます。

これはロードマップで最初に定めた複数テーブルのスライスです。

リレーション、テーブル間インデックス、別のストレージ形式は追加しません。

## ファイルシステム契約

`Catalog::from_path` は存在するディレクトリを受け取ります。

拡張子が大文字小文字を問わず `.dbf` である通常ファイルを、一つのテーブルとして扱います。

テーブル名はファイル名の拡張子を除いた部分です。

たとえば次の構成です。

```text
database/
├── comments.dbf
├── posts.dbf
├── posts.dbt
└── users.dbf
```

カタログは `comments`、`posts`、`users` を公開します。

memo サイドカー、WAL ファイル、ロックファイル、無関係なファイルはカタログ項目になりません。

検出するのは直下のファイルだけです。

公開 API ではテーブル名を完全一致で照合します。

カタログはマニフェストを書き込まず、ファイル名を変更せず、サイドカーの所有権も主張しません。

これにより DBF ファイルを既存の xBase ツールで読み取れ、テーブル単位の復旧は既存の DBF 経路に任せられます。

## Rust API

```rust
use txbase::catalog::Catalog;

let catalog = Catalog::from_path("database")?;
let users = catalog.open_table("users")?;
let names = catalog.table_names();
catalog.verify()?;
```

`open_table` は既存の復旧および memo サイドカー経路を通して一つの DBF をロードします。

`table_path` はテーブルをロードせずにパスを返します。

`schema_json` は検出した全テーブルをロードし、名前、ファイル名、DBF スキーマを返します。

`transaction_id` は、存在すれば最後に永続化したカタログジャーナルのコミット ID を返します。

この ID が進むのは複数テーブルの `POST /transaction` ジャーナル境界だけであり、独立した名前付きテーブルの更新はテーブルごとの DBF トランザクション ID を保ちます。

`verify` は検出した全テーブルをロードして検証し、失敗したテーブル名を報告します。

有界ローカル結合の境界はカタログ検出とは別です。

検証済みの結合文書と `Catalog` を `txbase::query::join::execute` に渡すと、永続的な関係マニフェストを追加せずに一つ以上の名前付きテーブルを読み取れます。

任意のカタログサーバーはこの境界を HTTP で公開します。

```bash
txbase --serve-catalog path/to/database --bind 127.0.0.1:8080
```

`GET` または `HEAD /catalog` は検出したテーブルスキーマと、カタログ表現を示す強い `ETag` を返します。

強いタグまたは弱いタグが一致する `If-None-Match`、あるいは `*` を指定すると、本文なしの `304 Not Modified` を返します。

`GET` または `HEAD /{table}/records` と `/{table}/records/{id}` は、現在の表現 `ETag` を含む単一テーブルのレコード応答意味論を再利用します。

`QUERY /{table}/records` は単一テーブル経路と同じ JSON クエリ文書を受け付けます。

`QUERY /{table}/records/stream` は `filter`、`projection`、`skip`、`limit` に対応する有界 `application/x-ndjson` ストリームを公開します。

`QUERY /{table}/explain` は単一テーブルの `QUERY /explain` と同じ選択済み計画を返します。

`QUERY /join` は `query::join::parse` と同じ JSON 結合文書を受け付け、JSON 配列を返し、結合ステージごとに 100,000 行の上限を維持します。

`POST /{table}/records` と `PUT`、`PATCH`、`DELETE /{table}/records/{id}` は、単一テーブルの更新、WAL、ETag、検証、制約の動作を再利用します。

各リクエストは名前付き DBF だけをコミットします。

`POST /transaction` は名前付きテーブルの更新パスを受け取り、カタログジャーナルの下で影響を受ける全 DBF をコミットします。

未完了の準備は次のカタログ読み取り時にロールバックします。

成功したカタログジャーナルコミットは永続的なカタログトランザクション ID を進め、JSON 本文と `X-Txbase-Transaction-Id` ヘッダーで返します。

応答は新しいカタログ表現 `ETag` も返します。

任意の `If-Match` と `If-None-Match` はカタログ write lock の内側で評価します。

`If-Match` は現在の強いタグまたは `*` を要求し、弱いタグまたは一致しない値には `412 Precondition Failed` を返します。

強いタグまたは弱いタグが一致する `If-None-Match`、あるいは `*` にも同じ応答を返し、DBF やサイドカーを変更しません。

これは順序と識別の境界であり、MVCC の可視性ではありません。

カタログの検出はサーバー起動時に一度行います。

各リクエストは既存の復旧経路を通して名前付きテーブルをロードします。

名前付きテーブルの更新は、テーブルを追加または削除しません。

不正な DBF があっても、検出はファイルの識別だけを行うため、ディレクトリ検出自体は失敗しません。

不正なテーブルは `open_table`、`schema_json`、`verify` が報告します。

## CLI

カタログと全テーブルのスキーマを表示します。

```bash
txbase catalog path/to/database
```

検出した全テーブルを検証します。

```bash
txbase verify-catalog path/to/database
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

完全なネストスキーマには、DBF ヘッダー、レコード数、memo サイドカーの検出結果、フィールド記述子が含まれます。

## 境界

カタログは現在、検出、検索、スキーマの内省、検証、有界ローカル結合が使う入力境界、名前付きテーブルを独立して読み書きする任意の HTTP 境界を提供します。

名前付きレコードの更新を横断するアトミックなトランザクション境界も提供します。

カタログジャーナルは単調増加するコミット ID を `.txbase.catalog.state` に永続化します。

準備済みジャーナルでは状態をロールバックし、コミット済みジャーナルでは状態を再適用して復旧します。

過去の行バージョンや MVCC の可視性は提供しません。

フィールドサイドカーが `references: "TABLE.FIELD"` を宣言すると、カタログの名前付き更新とカタログトランザクションは、非 null の子値が参照テーブルのアクティブ行に存在するか検証します。

子を孤立させる親の更新や論理削除は拒否します。

null は許可し、変更を連鎖させません。

直接の単一テーブル経路はテーブル間規則を解決できません。

カタログロックはカタログの読み書きを直列化し、テーブル単位のロックは直接の DBF 永続化を保護します。

フィールド名からリレーションを推測することはありません。

ローカル結合は `inner`、`left`、`right`、`semi`、`anti` の等値結合と、有界な `cross` 結合をサポートし、結果数に上限を持ちます。

完全なコストベースプランナー、インデックス対応またはマージ結合戦略、ストリーミングのバックプレッシャー、MVCC 可視性は提供しません。
