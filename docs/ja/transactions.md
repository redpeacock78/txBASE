# スナップショットトランザクション

`txbase::dbf::DbfTransaction`は、パスで指定した1つのDBFテーブルに対する公開オプティミスティックトランザクション境界です。

1つのテーブルスナップショットを読み込み、非公開コピーへ操作を適用し、成功した更新を1つのWAL付き保存で公開します。

この文書では、現在のAPI境界と意図的に制限している分離境界を定義します。

## 1. ライフサイクル

### 開始

`DbfTransaction::begin(path)`は現在のDBFテーブルを読み込み、後のcommit検査に使う元の状態を記録します。

トランザクションはcommitまたはrollbackされるまで、そのテーブルコピーを所有します。

`DbfTransaction::begin_serializable(path)`は、粗粒度のserializable境界を選択します。
テーブルの排他ロックを読み込み前に取得し、commit、rollback、dropまで保持します。
同じロックを使う他のローカルtxBASE DBF読み書きは、トランザクションがアクティブな間待機します。
この境界は1つのDBFパスに適用され、カタログをロックせず、txBASEを迂回する外部書き込みとは調整しません。

### 適用

`apply(&OperationIr)`は、HTTPトランザクション経路が使うものと同じ操作表現を受け付けます。

操作は`commit`が成功するまで、非公開のテーブルコピーだけを変更します。

### クエリ

`query(&QueryRequest)`と`QueryExecutor`実装は、非公開スナップショットに対してクエリを評価します。

そのため、commit前の更新はトランザクションの呼び出し元から見えますが、別のテーブル読み込みからは見えません。

### commit

`commit()`は、既存のテーブルロック、WAL、MVCC、サイドカー、トランザクション状態の永続化経路を通して、非公開テーブルを保存します。

結果は1つのテーブルcommitとなり、返される`DbfTable`には永続的なトランザクションIDが入ります。

`begin_serializable`で開始したトランザクションでは、`commit()`は既に保持しているロックを再利用し、ロックを再取得しません。

`begin`の後にDBF、memo、スキーマ、トランザクション状態の元データが変わっていた場合、commitは新しい状態を上書きせず拒否されます。

`commit_with_row_merge()`は、より狭い競合契約を明示的に選ぶためのAPIです。
テーブルロック内で現在のイメージを再読み込みし、現在のイメージ側に異なる変更がない物理レコードに対して、このトランザクションの更新または削除だけを適用します。
結果が同じ行は追加の変更がないものとして扱います。
insert、同一行への異なる変更、レコード件数の変更、レイアウトの変更、スキーマの変更は拒否します。
マージ後の結果も、同じWAL、MVCC、サイドカー、トランザクション状態の永続化経路を使います。

### rollback

`rollback()`はテーブルのパスへ書き込まず、非公開コピーを破棄します。

`commit`を呼ばずにトランザクションをdropした場合も、永続化に関して同じ結果になります。

## 2. 可視性と競合処理

既定のトランザクションはオプティミスティックです。

`begin`から`commit`までテーブルロックを保持することはありません。

`DbfTransaction::begin_serializable`は、beginからcommitまたはrollbackまで、テーブル単位の排他ロックによって実行を直列化します。

commit時の元データ検査が、独立して読み込んだテーブルスナップショットに対する書き込み競合の境界になります。

既定の`commit()`で競合した場合、呼び出し元は失敗したトランザクションを破棄し、現在のテーブルを再読み込みして操作を再試行するか判断しなければなりません。

アプリケーションの契約が、互いに異なる物理レコードへの更新または削除に限られる場合だけ、呼び出し元は`commit_with_row_merge()`を明示的に選べます。

APIはマージ方針を自動選択せず、ネットワーク応答を失った場合のexactly-once効果も保証しません。

## 3. 他のトランザクション境界との関係

単一テーブルのHTTP `POST /transaction`経路は、この適用とcommitの境界を再利用します。

カタログの`POST /transaction`経路は、カタログロックの下でDBFとサイドカーのイメージを調整するため、テーブル間ジャーナルの別境界として残ります。

`src/transaction/`エンジンは低レベルのWALトランザクションプリミティブとして残り、DBFの可視性やこのテーブルAPIは提供しません。

テーブルとカタログの過去スナップショットは、引き続き読み取り専用のMVCCビューです。

テーブル横断の安定した読み取りイメージが必要な場合、`Catalog::begin_read`は、すべての検出済みテーブルを取得してからロックを解放する`CatalogReadTransaction`を返します。

このAPIは読み取り専用で永続化されません。

保持されたcommitを再び開く境界は、引き続きカタログMVCC APIです。

## 4. 分離境界

既定のAPIは、1つのDBFテーブルに対する非公開スナップショットと、commit時のオプティミスティックな古い元データの拒否を提供します。

`DbfTransaction::begin_serializable`は、beginからcommitまたはrollbackまでテーブル単位の排他ロックを保持し、1つのテーブルに対する厳密な直列実行を提供します。
これは意図的にテーブル全体をロックするため、述語単位の並行性やテーブル横断のserializableトランザクションは提供しません。

`CatalogReadTransaction`は、テーブル横断の読み取りと有界結合に対する別のメモリ内スナップショット境界を提供します。

既定のAPIは、述語ロックまたは述語単位のserializable競合検出を提供しません。
`commit_with_row_merge()`は、明示的な物理レコードの更新および削除のマージに限られ、述語またはserializableの意味論は提供しません。

述語単位のロック、テーブル横断のserializable検証、カタログの独立した保持、長寿命の分散トランザクションは、将来の作業です。

## 5. 例

```rust
use txbase::dbf::DbfTransaction;
use txbase::query::{parse, QueryExecutor};

let mut transaction = DbfTransaction::begin("users.dbf")?;
transaction.apply(&operation)?;
let rows = transaction.execute(&parse(br#"{}"#)?)?;
let committed_table = transaction.commit()?;
```

この狭い物理レコード競合の契約で十分な場合だけ、`commit_with_row_merge()`を使います。

HTTP経路とこのRust APIは、同じテーブル更新および永続化経路を共有するため、動作を変更するときはAPIテストとHTTP契約テストの両方を更新しなければなりません。

## 主な参照先

- [PostgreSQLのトランザクション分離](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQLの並行性制御](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLiteの分離](https://sqlite.org/isolation.html)
- [SQLiteのWAL](https://sqlite.org/wal.html)
- [MVCCと過去スナップショット](mvcc.md)
- [更新モデル](mutation-model.md)
- [HTTPメソッドの意味とQUERY](http-semantics.md)
