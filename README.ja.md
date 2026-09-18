# txbase

`txbase`は、dBASE互換のDBFを中心に据えたRust製データベースの初期実装です。

現在の実装は、DBFの読み取り、JSON出力、HTTPの`GET`、RFC 10008に基づく`QUERY`の実行に範囲を限定しています。

WAL、MVCC、更新処理、xBase互換フロントエンドは、後続工程の境界だけを定義しています。

## 現在できること

- classic DBFの32バイトfield descriptorを読み取る。
- dBASE Level 7の48バイトfield descriptorを判別する。
- header、field descriptor、record、削除フラグを読み取る。
- 代表的な文字列、日付、数値、論理値、整数、倍精度値をJSONへ変換する。
- コマンドラインから有効なrecordをJSONとして出力する。
- `GET /records`と`GET /records/{id}`を提供する。
- `QUERY /records`でfilter、sort、projection、skip、limitを実行する。

DBFのlanguage-driver byteは保持しますが、OEM code pageとWindows code pageの完全な変換はまだ実装していません。

## 最短の手順

DBFをJSONとして読み取ります。

```bash
cargo run -- path/to/users.dbf
```

HTTP serverを起動します。

```bash
cargo run -- --serve path/to/users.dbf
```

1-basedのDBF record numberでrecordを取得します。

```bash
curl -s http://127.0.0.1:8080/records/1 | jq
```

有効なrecordを一覧します。

```bash
curl -s http://127.0.0.1:8080/records | jq
```

`QUERY`は`Content-Type: application/json`を要求します。

`QUERY`はactive recordに対してfilter、sort、projection、skip、limitを適用します。

```bash
curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"limit":10}' \
  http://127.0.0.1:8080/records
```

listener addressは`--bind ADDRESS`で変更できます。

## 構成

```text
src/dbf.rs          DBF parserとJSON変換
src/query.rs        JSON query documentとexecutor
src/server.rs       HTTP routingとQUERY境界
src/storage.rs      range-based storage境界
src/transaction.rs  WAL、MVCC、transaction境界
src/xbase.rs        共通operation IR境界
tests/fixtures/     parser test用fixture
docs/research.md    仕様調査と設計判断
```

workspaceは、責務の所有者やbuild上の理由が生じるまで単一packageで保ちます。

## Query実行

query documentはMongoDBのpredicateから必要な表現だけを借りています。

```json
{
  "filter": {
    "AGE": {"$gte": 20, "$lt": 30},
    "COUNTRY": {"$in": ["JP", "TW"]}
  },
  "sort": {"AGE": 1},
  "projection": {"NAME": 1, "AGE": 1},
  "limit": 100,
  "skip": 0
}
```

初期operatorは`$eq`、`$ne`、`$gt`、`$gte`、`$lt`、`$lte`、`$in`、`$nin`、`$and`、`$or`、`$not`です。

missing fieldでは`$ne`と`$nin`が一致し、array valueでは要素のいずれかが条件を満たすと一致します。
sortの同値recordはDBF record orderを保ちます。
Dotted path、index、update operatorは未対応です。

## 検証

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

仕様調査の詳細は[docs/research.md](docs/research.md)に記録しています。

英語の概要は[README.md](README.md)です。
