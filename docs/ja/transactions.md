# スナップショットトランザクション

`txbase::dbf::DbfTransaction`は、パスで指定した1つのDBFテーブルに対する公開オプティミスティックトランザクション境界です。

1つのテーブルスナップショットを読み込み、非公開コピーへ操作を適用し、成功した更新を1つのWAL付き保存で公開します。

この文書では、現在のAPI境界と意図的に制限している分離境界を定義します。

## 1. ライフサイクル

### 開始

`DbfTransaction::begin(path)`は現在のDBFテーブルを読み込み、後のcommit検査に使う元の状態を記録します。

トランザクションはcommitまたはrollbackされるまで、そのテーブルコピーを所有します。

### 適用

`apply(&OperationIr)`は、HTTPトランザクション経路が使うものと同じ操作表現を受け付けます。

操作は`commit`が成功するまで、非公開のテーブルコピーだけを変更します。

### クエリ

`query(&QueryRequest)`と`QueryExecutor`実装は、非公開スナップショットに対してクエリを評価します。

そのため、commit前の更新はトランザクションの呼び出し元から見えますが、別のテーブル読み込みからは見えません。

### commit

`commit()`は、既存のテーブルロック、WAL、MVCC、サイドカー、トランザクション状態の永続化経路を通して、非公開テーブルを保存します。

結果は1つのテーブルcommitとなり、返される`DbfTable`には永続的なトランザクションIDが入ります。

`begin`の後にDBF、memo、スキーマ、トランザクション状態の元データが変わっていた場合、commitは新しい状態を上書きせず拒否されます。

### rollback

`rollback()`はテーブルのパスへ書き込まず、非公開コピーを破棄します。

`commit`を呼ばずにトランザクションをdropした場合も、永続化に関して同じ結果になります。

## 2. 可視性と競合処理

このトランザクションはオプティミスティックです。

`begin`から`commit`までテーブルロックを保持することはありません。

commit時の元データ検査が、独立して読み込んだテーブルスナップショットに対する書き込み競合の境界になります。

競合した場合、呼び出し元は失敗したトランザクションを破棄し、現在のテーブルを再読み込みして操作を再試行するか判断しなければなりません。

APIは同時操作を自動的にマージせず、ネットワーク応答を失った場合のexactly-once効果も保証しません。

## 3. 他のトランザクション境界との関係

単一テーブルのHTTP `POST /transaction`経路は、この適用とcommitの境界を再利用します。

カタログの`POST /transaction`経路は、カタログロックの下でDBFとサイドカーのイメージを調整するため、テーブル間ジャーナルの別境界として残ります。

`src/transaction/`エンジンは低レベルのWALトランザクションプリミティブとして残り、DBFの可視性やこのテーブルAPIは提供しません。

テーブルとカタログの過去スナップショットは、引き続き読み取り専用のMVCCビューです。

## 4. 分離境界

現在のAPIは、1つのDBFテーブルに対する非公開スナップショットと、commit時のオプティミスティックな古い元データの拒否を提供します。

述語ロック、serializableな競合検出、行単位の書き込み競合マージ、テーブル間トランザクションオブジェクトは提供しません。

独立した行保持方針と長寿命の分散トランザクションは、将来の作業です。

## 5. 例

```rust
use txbase::dbf::DbfTransaction;
use txbase::query::{parse, QueryExecutor};

let mut transaction = DbfTransaction::begin("users.dbf")?;
transaction.apply(&operation)?;
let rows = transaction.execute(&parse(br#"{}"#)?)?;
let committed_table = transaction.commit()?;
```

HTTP経路とこのRust APIは、同じテーブル更新および永続化経路を共有するため、動作を変更するときはAPIテストとHTTP契約テストの両方を更新しなければなりません。

## 主な参照先

- [PostgreSQLのトランザクション分離](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQLの並行性制御](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLiteの分離](https://sqlite.org/isolation.html)
- [SQLiteのWAL](https://sqlite.org/wal.html)
- [MVCCと過去スナップショット](mvcc.md)
- [更新モデル](mutation-model.md)
- [HTTPメソッドの意味とQUERY](http-semantics.md)
