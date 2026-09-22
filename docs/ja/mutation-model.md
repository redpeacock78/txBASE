# 更新モデル

この文書は、ローカルの更新文書とその境界を定義します。

JSONの更新形式はtxBASE内のアプリケーション契約です。

この形式だけでHTTP `PATCH`を定義することも、MongoDB互換性を主張することもありません。

## 1. 型付き更新演算子

`PATCH`は通常のフィールドオブジェクトまたは型付き更新文書を受け付けます。

現在の型付きサブセットは次のとおりです。

```json
{
  "$set": {"NAME": "Caroline"},
  "$unset": {"TEMP": true},
  "$inc": {"COUNT": 1}
}
```

更新文書で演算子と通常のフィールドを混在できません。

1つの更新文書で同じフィールドを複数回変更できません。

未知のフィールドと自動インクリメントフィールドへの書き込みは拒否します。

未サポートの演算子はフィールド名として扱わず、拒否します。

MongoDBには配列演算子や算術演算子を含む、より大きな[更新演算子リファレンス](https://www.mongodb.com/docs/manual/reference/mql/update/)があります。

これらは現在のtxBASE契約には含まれません。

## 2. 更新とアトミック性

HTTPの冪等性だけではDBFの書き込み経路をクラッシュ安全にはできません。

現在の更新経路は、状態ペイロードの前に永続的な`TXOP`意図を記録し、WALを同期してからDBFまたはmemoサイドカーを置き換えます。

起動時の復旧は、状態ペイロードがない場合にサポート対象の意図を再実行します。

`POST /transaction`はすべての操作を非公開のテーブルコピーへ適用し、1つのスナップショットまたはWALコミットとして永続化します。

公開Rust `DbfTransaction`は、同じ単一テーブルの適用、クエリ、古い元データの検査、commitの境界を公開し、HTTP経路はそれを再利用します。

検証または操作のいずれかが失敗した場合はコピーを破棄し、現在のDBFを変更しません。

コミットIDはそのDBFに対して永続化され、再起動またはWAL復旧後も継続します。

この単一テーブル境界は、テーブル間アトミック性とカタログ全体のトランザクションIDを提供しません。
`mvcc` CLIと`*.txbase.mvcc`サイドカーを通じて、テーブル単位の過去スナップショットを提供します。

カタログサーバーの`POST /transaction`は独立したテーブル間境界です。

これはカタログロックの下で対象テーブルを準備し、DBFと変更されたサイドカーをディレクトリジャーナルでコミットし、次のカタログ読み取り時に未完了の準備をロールバックします。

永続的なカタログジャーナルのトランザクションIDをJSONと`X-Txbase-Transaction-Id`で返します。

このIDはカタログコミットの識別と順序付けに使います。

過去の複数テーブルに対するMVCC可視性は提供しません。

テーブルロックは保存経路を直列化し、独立してロードされた古いテーブルは拒否します。

自動ネットワーク再試行、リクエスト重複排除、複数の書き手によるマージは実装していません。

成功したTCP交換だけからexactly-onceの効果を推測してはいけません。

RustトランザクションAPIのライフサイクルと分離の制限は、[スナップショットトランザクション](transactions.md)で定義します。

## 3. HTTP の境界

[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html)は、`PATCH`をリソースへパッチ文書を適用するメソッドとして定義します。

パッチ文書のメディアタイプと処理規則はAPI契約の一部です。

`PATCH`は既定では安全でも冪等でもありません。

APIは文書の意味論または条件付きリクエストによって特定のパッチを冪等にできます。

衝突に敏感なパッチでは、RFCは強いエンティティタグを使う`If-Match`などの条件付きリクエストを推奨します。

txBASEは`application/json`の通常フィールドパッチと、上記の型付きサブセットを受け付けます。

レコードの`PATCH`には`application/merge-patch+json`も受け付けます。

Merge Patch文書のルートはオブジェクトでなければなりません。

オブジェクトのメンバーは再帰的にマージし、`null`はメンバーを削除し、配列とスカラー値は現在値を置き換えます。

DBFレコードは固定されたスキーマのスカラー値を持つため、既知のフィールドの削除は正規化された`null`値として保存し、スカラー項目へのオブジェクト値は拒否します。

レコードの`PATCH`には`application/json-patch+json`も受け付けます。

JSON Patch文書は、RFC 6902の操作を100件まで持つ配列です。

txBASEはRFC 6901のJSON Pointerパスを使う`add`、`remove`、`replace`、`test`、`move`、`copy`を実装します。

レコードのルートはオブジェクトでなければなりません。

既知のDBFフィールドを削除すると正規化した`null`として永続化し、操作または最終DBF検証に失敗した場合は`422`を返して更新を適用しません。

状態変更経路に強いテーブル表現タグと任意の`If-Match`保護を実装しています。

GETとHEADは`If-None-Match`によるキャッシュ検証を提供し、単一テーブルの更新経路は同じ条件の弱い比較が一致した場合に`412 Precondition Failed`を返します。

catalog全体のトランザクション経路はカタログ表現`ETag`を公開します。

任意の`If-Match`と`If-None-Match`の条件はcatalog write lockの内側で評価します。

`If-Match`は現在の強いタグまたは`*`を要求し、強いタグまたは弱いタグが一致する`If-None-Match`、あるいは`*`は、変更なしの`412 Precondition Failed`を返します。

成功したコミットは新しいタグを返します。

MongoDB風の更新文書はJSON本文内のアプリケーション形式です。

それ自体がHTTP `PATCH`の定義ではありません。

## 4. MongoDB ワイヤープロトコルではない

txBASEはBSON、MongoDBワイヤープロトコル、JavaScript式、MongoDBの照合、MongoDBの集約パイプライン、MongoDBのインデックス、完全な更新演算子の集合を実装しません。

JSON構文は意図的に小さなローカルAPIです。

[MongoDB のアトミック性とトランザクションのガイド](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)は、単一レコードの更新と将来の複数レコードトランザクション保証を分ける参考になります。

`$inc`や現在のHTTP `PATCH`経路から複数レコードのアトミック性を推測してはいけません。

## 5. 今後の更新作業

実装前に次の項目には明示的な契約が必要です。

- 複数レコード更新の意味論と可視性規則。
- 小さな更新参照モデルとの微分テスト。

## 主な参照先

- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [MongoDB update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [MongoDB atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [HTTP メソッドの意味](http-semantics.md)
