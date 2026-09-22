# 変更データ取得

txBASEは、単一テーブルのコミット済み行変更を永続CDCサイドカーへ記録します。

このサイドカーは、ローカル利用者が変更を観測するための境界です。

レプリケーションプロトコルや配送キューではありません。

PostgreSQLの論理デコードとの互換性も主張しません。

## 1. サイドカーとイベント形式

`users.dbf`のCDCサイドカーは`users.txbase.cdc`です。

サイドカーは既存の`TXWL`レコード容器を使い、コミット済みDBFトランザクションごとに1つの`TXCD`ペイロードを持ちます。

`TXCD`バージョン1は、次の形のJSONイベントを持ちます。

```json
{
  "transaction_id": 2,
  "reset": false,
  "changes": [
    {
      "record_number": 1,
      "before": {
        "deleted": false,
        "values": {"ID": 1, "NAME": "Alice"}
      },
      "after": {
        "deleted": false,
        "values": {"ID": 1, "NAME": "Bob"}
      }
    }
  ]
}
```

`transaction_id`は、DBF WALと同じトランザクション境界で永続化する正のDBFコミットIDです。

`changes`は、1始まりの物理DBFレコード番号で並びます。

挿入では`before`状態を持ちません。

論理削除では`after`状態を残し、その`deleted`を`true`にします。

コミット済み操作が行の値を変えない場合は、`changes`が空でも有効です。

カタログトランザクションは、専用のカタログサイドカーを使います。

カタログディレクトリのサイドカーは`.txbase.catalog.cdc`です。

単一テーブルと同じ`TXWL`レコード容器を使い、コミット済みの複数テーブルカタログトランザクションごとに1つの`TXCC`ペイロードを持ちます。

`TXCC`バージョン1は、1つのカタログトランザクションIDとテーブル名のマップを持ちます。

```json
{
  "transaction_id": 1,
  "tables": {
    "posts": {
      "reset": false,
      "changes": [
        {
          "record_number": 1,
          "before": {"deleted": false, "values": {"AGE": 29}},
          "after": {"deleted": false, "values": {"AGE": 30}}
        }
      ]
    },
    "users": {
      "reset": false,
      "changes": [
        {
          "record_number": 3,
          "before": null,
          "after": {"deleted": false, "values": {"ID": 3, "NAME": "Carol"}}
        }
      ]
    }
  }
}
```

テーブルのマップはテーブル名で並び、各テーブルの変更は物理レコード番号で並びます。

カタログトランザクションIDは、カタログのスキーマJSONと`POST /transaction`が公開するIDと同じです。

## 2. コミットと復旧

CDCペイロードは、対象ファイルを置き換える前にDBF WALへ書き込みます。

DBF、memo、インデックス、トランザクション状態、MVCCの対象をコミットした後で、CDCサイドカーへイベントを公開します。

テーブルのコミット後に公開が失敗した場合、WALを残し、次回の通常のDBF読み込みで同じイベントを再試行します。

同じトランザクションIDを持つイベントの再公開は、同じ変更を表す場合だけ冪等です。

既存トランザクションIDへの異なるイベント、順序が戻るトランザクションID、壊れた`TXCD`ペイロードは拒否します。

CDCの読み取りは、WAL読み取りと同じ壊れた末尾の境界を受け入れ、サイドカーを開くときに不完全な末尾だけを削除します。

復旧処理はCDCのトランザクションIDと`TXTI`を照合し、DBFの復旧が完了した後でイベントを公開します。

カタログジャーナルの復旧は、DBF、インデックス、MVCC、トランザクション状態の対象と一緒にカタログCDCサイドカーを適用またはロールバックします。

同じトランザクションIDとイベントデータを持つカタログCDCの公開は冪等であり、競合または順序外のイベントは拒否します。

## 3. レイアウト変更

`PACK`のように物理レイアウトを変更してレコード番号の同一性を無効にしうる場合、`reset`は`true`になります。

このイベントは、影響するレコード番号の範囲について変更前と変更後の状態を持ちます。

利用者は、resetイベントを各レコード番号へ個別に適用するのではなく、置換境界として扱わなければなりません。

現在のイベントはスキーマサイドカーの編集を含みません。

カタログCDCは、明示的な複数テーブルカタログトランザクションの境界を対象にします。

独立した名前付きテーブル更新の経路はテーブル単位の`TXCD`イベントを保持し、カタログイベントへ結合しません。

## 4. イベントの読み取り

Rust APIは、指定したトランザクションIDより後のコミット済みイベントを返します。

```rust
use txbase::dbf::DbfTable;

let events = DbfTable::cdc_events("users.dbf", Some(10))?;
```

カタログAPIは、原子的な複数テーブルイベントを返します。

```rust
use txbase::catalog::Catalog;

let events = Catalog::cdc_events("catalog", Some(10))?;
```

CLIでも同じ読み取り境界を利用できます。

```bash
txbase cdc users.dbf
txbase cdc users.dbf --after 10
txbase cdc catalog ./catalog --after 10
```

サイドカーがなければ、空のJSON配列を返します。

`--after`のカーソルは読み取り位置であり、確認応答や永続的な利用者スロットではありません。

CDCの保持は明示的なファイル保守であり、自動では行いません。

`backup`と`restore`は、DBFやほかのサポート対象サイドカーと一緒にCDCサイドカーを検証してコピーします。

## 5. HTTP読み取り転送

単一テーブルサーバーは、コミット済みのテーブルイベントを`GET /cdc`と`HEAD /cdc`で公開します。

カタログサーバーは、コミット済みの複数テーブルイベントを`GET /cdc`と`HEAD /cdc`で公開します。

カタログのルートは、カタログサイドカーにある`TXCC`イベントを返します。

独立した名前付きテーブル更新の`TXCD`イベントは、1つのカタログトランザクションを表さないため結合しません。

両方のルートは、任意の排他的なトランザクションIDカーソル`after`と、`limit`クエリパラメーターを受け付けます。

既定のページサイズは100イベントで、最大は1,000イベントです。

応答は、有界な次のJSONオブジェクトです。

```json
{
  "events": [],
  "next_after": 12
}
```

サイドカーの末尾まで到達した場合、`next_after`は`null`です。

クライアントは`after=next_after`を渡して次のページを要求できます。

サイドカーがなければ、空の`events`配列と`null`のカーソルを返します。

値が0または数値でないパラメーター、重複したパラメーター、未知のクエリパラメーターは`400`で拒否します。

`GET`と`HEAD`は読み取り専用であり、利用者の確認応答や読み取り位置の保持を行いません。

## 6. 範囲と今後の作業

イベントは1つのDBFコミットまたは1つのカタログコミットにおける状態差分を記録し、コミット順序を保ちます。

ネットワーク転送、利用者リース、確認応答、バックプレッシャー、スキーマ進化、別テーブルへの再生は提供しません。

レプリケーションや外部利用者向けのアダプターの動作は、別途定義します。

SQLiteの[セッション拡張](https://www.sqlite.org/sessionintro.html)は、変更セットの形と競合を扱う変更転送の参考資料です。

PostgreSQLの[論理デコード](https://www.postgresql.org/docs/current/logicaldecoding.html)は、WALからコミット済み変更を導出して外部利用者へ公開する設計の参考資料です。

これらのシステムはこのローカルサイドカーより広い契約を持ち、txBASEがそれらの形式やプロトコルと互換になることを意味しません。

## 一次資料と範囲

- [SQLiteセッション拡張](https://www.sqlite.org/sessionintro.html)
- [PostgreSQL論理デコード](https://www.postgresql.org/docs/current/logicaldecoding.html)
- [DBF互換性の境界](dbf-compatibility.md)
- [スナップショットトランザクション](transactions.md)
- [MVCCと履歴スナップショット](mvcc.md)

外部文書は用語と設計上の着眼点を提供します。

`TXCD`と`TXCC`のペイロード、物理レコードのreset規則、サイドカーのパス、復旧順序、CLIカーソルはtxBASEの契約です。
