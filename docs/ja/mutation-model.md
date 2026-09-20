# 更新モデル

この文書は、ローカルの更新文書とその境界を定義します。

JSON の更新形式は txBASE 内のアプリケーション契約です。

この形式だけで HTTP `PATCH` を定義することも、MongoDB 互換性を主張することもありません。

## 1. 型付き更新演算子

`PATCH` は通常のフィールドオブジェクトまたは型付き更新文書を受け付けます。

現在の型付きサブセットは次のとおりです。

```json
{
  "$set": {"NAME": "Caroline"},
  "$unset": {"TEMP": true},
  "$inc": {"COUNT": 1}
}
```

更新文書で演算子と通常のフィールドを混在させることはできません。

一つの更新文書で同じフィールドを複数回変更することはできません。

未知のフィールドと自動インクリメントフィールドへの書き込みは拒否します。

未サポートの演算子はフィールド名として黙って扱わず拒否します。

MongoDB には配列演算子や算術演算子を含む、より大きな[更新演算子リファレンス](https://www.mongodb.com/docs/manual/reference/mql/update/)があります。

これらは現在の txBASE 契約には含まれません。

## 2. 更新とアトミック性

HTTP の冪等性だけでは DBF の書き込み経路をクラッシュ安全にはできません。

現在の更新経路は、状態ペイロードの前に永続的な `TXOP` 意図を記録し、WAL を同期してから DBF または memo サイドカーを置き換えます。

起動時の復旧は、状態ペイロードがない場合にサポート対象の意図を再実行します。

`POST /transaction` はすべての操作を非公開のテーブルコピーへ適用し、一つのスナップショットまたは WAL コミットとして永続化します。

検証または操作のいずれかが失敗した場合はコピーを破棄し、現在の DBF を変更しません。

コミット ID はその DBF に対して永続化され、再起動または WAL 復旧後も継続します。

この単一テーブル境界は、テーブル間アトミック性、カタログ全体のトランザクション ID、MVCC 可視性を提供しません。

カタログサーバーの `POST /transaction` は独立したテーブル間境界です。

これはカタログロックの下で対象テーブルを準備し、DBF と変更されたサイドカーをディレクトリジャーナルでコミットし、次のカタログ読み取り時に未完了の準備をロールバックします。

永続的なカタログジャーナルのトランザクション ID を JSON と `X-Txbase-Transaction-Id` で返します。

この ID はカタログコミットの識別と順序付けに使います。

MVCC の可視性は提供しません。

テーブルロックは保存経路を直列化し、独立してロードされた古いテーブルは拒否します。

自動ネットワーク再試行、リクエスト重複排除、複数の書き手によるマージは実装していません。

成功した TCP 交換だけから exactly-once の効果を推測してはいけません。

## 3. HTTP の境界

[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html) は、`PATCH` をリソースへパッチ文書を適用するメソッドとして定義します。

パッチ文書のメディアタイプと処理規則は API 契約の一部です。

`PATCH` は既定では安全でも冪等でもありません。

API は文書の意味論または条件付きリクエストによって特定のパッチを冪等にできます。

衝突に敏感なパッチでは、RFC は強いエンティティタグを使う `If-Match` などの条件付きリクエストを推奨します。

txBASE は `application/json` の通常フィールドパッチと、上記の型付きサブセットを受け付けます。

状態変更経路に強いテーブル表現タグと任意の `If-Match` 保護を実装しています。

GET と HEAD は `If-None-Match` によるキャッシュ検証を提供し、単一テーブルの更新経路は同じ条件の弱い比較が一致した場合に `412 Precondition Failed` を返します。

catalog 全体の transaction endpoint は catalog representation ETag をまだ持ちません。
JSON Patch と JSON Merge Patch のメディアタイプは未実装です。

MongoDB 風の更新文書は JSON 本文内のアプリケーション形式です。

それ自体が HTTP `PATCH` の定義ではありません。

## 4. MongoDB ワイヤープロトコルではない

txBASE は BSON、MongoDB ワイヤープロトコル、JavaScript 式、MongoDB の照合、MongoDB の集約パイプライン、MongoDB のインデックス、完全な更新演算子集合を実装しません。

JSON 構文は意図的に小さなローカル API です。

[MongoDB のアトミック性とトランザクションのガイド](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)は、単一レコードの更新と将来の複数レコードトランザクション保証を分ける参考になります。

`$inc` や現在の HTTP `PATCH` 経路から複数レコードのアトミック性を推測してはいけません。

## 5. 今後の更新作業

実装前に次の項目には明示的な契約が必要です。

- ローカル更新文書に加える標準パッチメディアタイプ。
- catalog 全体の transaction endpoint に対する catalog representation ETag と validator。
- 複数レコード更新の意味論と可視性規則。
- 小さな更新参照モデルとの微分テスト。

## 主な参照先

- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [MongoDB update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [MongoDB atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [HTTP メソッドの意味](http-semantics.md)
