# HTTP メソッドの意味

txBASEは、定義された意味に従ってHTTPメソッド名を使います。

すべてのメソッドを任意のRPC動詞として扱うことはありません。

現在のAPIは意図的に小さく、JSON専用です。

## 1. メソッドの基本的な性質

[RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.html)は、クライアント、中間処理、再試行、キャッシュに影響する3つの性質を定義します。

**安全**とは、クライアントが状態変更操作を要求していないことです。

**冪等**とは、同じリクエストを繰り返しても一度だけ実行した場合と意図した効果が同じことです。

**キャッシュ可能**とは、メソッドと応答メタデータが許す場合に応答を保存して再利用できることです。

安全であっても、ログなどの付随的なサーバー副作用がないとは限りません。

一般的なメソッドの性質は次のとおりです。

| メソッド | 安全 | 冪等 | txBASE の役割 |
| --- | --- | --- | --- |
| `GET` | はい | はい | レコードまたは一つのレコードを読む |
| `HEAD` | はい | はい | レコードまたは一つのレコードのヘッダーを読む |
| `OPTIONS` | はい | はい | 対応メソッドと JSON クエリメディアタイプを通知する |
| `TRACE` | はい | はい | 未実装 |
| `POST` | いいえ | いいえ | 物理 DBF レコードを作成する |
| `PUT` | いいえ | はい | アクティブなレコードを置き換える |
| `PATCH` | 既定ではいいえ | 既定ではいいえ | アクティブなレコードを部分変更する |
| `DELETE` | いいえ | はい | DBF の削除マーカーを書き込む |
| `QUERY` | はい | はい | 本文を伴うクエリを実行する |

この性質はメソッドの意図を表します。

それだけでトランザクション分離、重複排除、再試行に安全なネットワーク実装が得られるわけではありません。

## 2. 現在の txBASE ルート

`OPTIONS`は`204 No Content`、サーバーの表面で対応するメソッドを示す`Allow`ヘッダー、`Accept-Query: "application/json"`、`Accept-Patch: application/json, application/merge-patch+json, application/json-patch+json`を返します。

この応答は、ルート規則が通常拒否するリソース上のメソッドを許可するものではありません。

| ルート | リクエスト契約 | 応答契約 |
| --- | --- | --- |
| `GET /records` | JSON 本文なし | アクティブレコードを JSON で返す |
| `GET /records/{id}` | 1 始まりの物理 DBF レコード番号 | アクティブな一つのレコード、または`404` |
| `HEAD /records`と`HEAD /records/{id}` | `GET`と同じ対象選択 | 本文なしで同じステータスと表現ヘッダー |
| `QUERY /records` | `Content-Type: application/json`とクエリ文書 | `Accept-Query`付きのフィルター済み JSON。ページング時は`records`と`cursor` |
| `QUERY /records/stream` | `Content-Type: application/json`とストリーム対応クエリ文書 | 一行一レコードの chunked `application/x-ndjson` |
| `GET /cdc`と`HEAD /cdc` | 任意の`after`と`limit`クエリパラメーター | `next_after`を伴う有界なコミット済み`TXCD`イベントページ |
| `QUERY /explain` | `Content-Type: application/json`とクエリ文書 | `Accept-Query`付きのテーブルスキャンまたはインデックス計画 |
| `GET /catalog`と`HEAD /catalog`（カタログサーバー） | JSON 本文なし | 強いカタログ`ETag`付きの検出したテーブルスキーマ。条件付きリクエストは`304`を返すことがある |
| `GET /cdc`と`HEAD /cdc`（カタログサーバー） | 任意の`after`と`limit`クエリパラメーター | `next_after`を伴う有界なコミット済み`TXCC`イベントページ |
| `GET`、`HEAD /{table}/records[/{id}]`（カタログサーバー） | JSON 本文なし | 名前付きテーブルのレコード |
| `QUERY /{table}/records`（カタログサーバー） | `Content-Type: application/json`とクエリ文書 | `Accept-Query`付きの名前付きテーブルのレコード |
| `QUERY /{table}/records/stream`（カタログサーバー） | `Content-Type: application/json`とストリーム対応クエリ文書 | 一行一レコードの chunked `application/x-ndjson` |
| `QUERY /{table}/explain`（カタログサーバー） | `Content-Type: application/json`とクエリ文書 | `Accept-Query`付きの名前付きテーブルのクエリ計画 |
| `QUERY /join`（カタログサーバー） | `Content-Type: application/json`と有界な結合文書 | `Accept-Query`付きの結合済み JSON 結果 |
| `POST /{table}/records`（カタログサーバー） | 既知のフィールドを持つ JSON オブジェクト | `201 Created`とテーブル修飾済み`Location` |
| `PUT`、`PATCH`、`DELETE /{table}/records/{id}`（カタログサーバー） | 単一テーブルと同じ本文および前提条件規則 | 独立した名前付きテーブル更新 |
| `POST /transaction`（カタログサーバー） | 名前付きテーブル更新操作を含む JSON オブジェクト | カタログジャーナルコミット後に新しいカタログ`ETag`付きの`200`、または失敗した`If-Match`と一致する`If-None-Match`に対する変更なしの`412` |
| `POST /records` | 既知のフィールドを持つ JSON オブジェクト | `201 Created`と`Location` |
| `POST /transaction` | 空でない`operations`配列を含む JSON オブジェクト | 一つのテーブルのアトミックスナップショットコミット後に`200` |
| `PUT /records/{id}` | フィールドを置き換える JSON オブジェクト | 結果のレコード |
| `PATCH /records/{id}` | `application/json`の更新文書、`application/merge-patch+json`のオブジェクト、または`application/json-patch+json`の配列 | 結果のレコード |
| `DELETE /records/{id}` | JSON 本文なし | `204 No Content` |

`PUT`には`application/json`が必要で、`PATCH`は`application/json`、`application/merge-patch+json`、`application/json-patch+json`を受け付けます。

未知のフィールド、不正なJSON、未サポートの更新演算子、不正なフィールド値は、永続化前に拒否します。

`DELETE`はDBFの論理削除です。

現在の更新層は物理レコード番号を再利用しません。

成功した`GET /records`と`GET /records/{id}`の応答は、現在のテーブル表現に対する強い`ETag`を公開します。

`POST /records`、`PUT /records/{id}`、`PATCH /records/{id}`、`DELETE /records/{id}`、`POST /transaction`は任意の`If-Match`ヘッダーを受け付けます。

強い現在タグ、または対象が存在する場合の`*`は更新を許可します。

弱いタグ、不正な条件、不一致の条件を指定した場合は、テーブルを変更せず`412 Precondition Failed`を返します。

ヘッダーを省略すれば既存の動作を保ちます。

カンマ区切りのリストは、いずれかの強いタグが一致すれば成功します。

成功した更新は新しい`ETag`を返します。

WAL付きの成功した更新は`X-Txbase-Transaction-Id`も返します。

`POST /transaction`は同じ正のDBFコミットIDをJSONの`transaction_id`メンバーでも返します。

バリデーターは不透明であり、真正性トークンや認可トークンではありません。

現在のメモリ上のDBFバイト列と解決済みのアクティブJSON値から導出するため、memoが保持する値も表現の識別に参加します。

`GET /records`と`HEAD /records`、およびそれぞれの`/{id}`形式は`If-None-Match`も受け付けます。

一致する強いタグまたは弱いタグ、あるいは存在するリソースに対する`*`は、現在の`ETag`と本文なしの`304 Not Modified`を返します。

一致しない値には通常の表現を返します。

`304`を返すこのキャッシュ検証は`GET`と`HEAD`に限られますが、更新経路は同じ弱い比較を使い、条件が一致した場合に`412 Precondition Failed`を返します。

`POST /records`、`PUT`、`PATCH`、`DELETE`、単一テーブルの`POST /transaction`は`If-None-Match`を受け付けます。

一致する強いタグまたは弱いタグ、あるいは存在する対象に対する`*`は、現在の`ETag`を伴う`412 Precondition Failed`を返し、更新しません。

一致しない条件は操作を許可します。

`/records` collectionと単一テーブルのtransaction targetは存在するリソースであるため、`*`はこれらの操作を拒否します。

`GET`と`HEAD /catalog`はカタログ表現を示す強い`ETag`を公開します。
強いタグまたは弱いタグが一致する`If-None-Match`、あるいは`*`は、現在の`ETag`と本文なしの`304 Not Modified`を返します。
catalog全体の`POST /transaction`は任意の`If-Match`と`If-None-Match`を受け付けます。
条件はcatalog write lockの内側で評価します。
`If-Match`は現在の強いタグか`*`を要求し、弱いタグや一致しない値には、DBFやサイドカーを変更せず、現在の`ETag`を伴う`412 Precondition Failed`を返します。
強いタグと弱いタグのどちらかが一致する`If-None-Match`、あるいは`*`にも同じ応答を返します。
一致しない条件はトランザクションを許可し、成功したコミットは新しいカタログ表現`ETag`を返します。

## 3. PATCH

[RFC 5789](https://www.rfc-editor.org/rfc/rfc5789.html)は、`PATCH`をリソースへパッチ文書を適用するメソッドとして定義します。

パッチ文書のメディアタイプと処理規則はAPI契約の一部です。

`PATCH`は既定では安全でも冪等でもありません。

APIは、文書の意味論または条件付きリクエストによって特定のパッチを冪等にできます。

衝突に敏感なパッチについてRFCは、強いエンティティタグを使う`If-Match`などの条件付きリクエストを推奨します。

txBASEは`application/json`の通常フィールドパッチと、[更新モデル](mutation-model.md)で定義する`$set`、`$unset`、`$inc`の型付きサブセットを受け付けます。

レコードの`PATCH`には`application/merge-patch+json`も受け付けます。

Merge Patch文書のルートはオブジェクトでなければなりません。

オブジェクトのメンバーは再帰的にマージし、`null`はメンバーを削除し、配列とスカラー値は現在値を置き換えます。

DBFレコードは固定されたスキーマのスカラー値を持つため、既知のフィールドの削除は正規化された`null`値として保存し、スカラー項目へのオブジェクト値は拒否します。

レコードの`PATCH`には`application/json-patch+json`も受け付けます。

JSON Patch文書は、RFC 6902の操作を100件まで持つ配列です。

txBASEはRFC 6901のJSON Pointerパスを使う`add`、`remove`、`replace`、`test`、`move`、`copy`を実装します。

レコードのルートはオブジェクトでなければなりません。

既知のDBFフィールドを削除すると正規化した`null`として永続化し、操作または最終DBF検証に失敗した場合は`422`を返して更新を適用しません。

上記の状態変更経路に対する強いテーブルおよびカタログ表現タグと、単一テーブル、名前付きテーブル、catalog全体のトランザクションに対する任意の`If-Match`保護および`If-None-Match`検証を実装しています。

MongoDB風の更新文書はJSON本文内のアプリケーション形式です。

それ自体がHTTP `PATCH`の定義ではありません。

## 4. QUERY

[RFC 10008](https://www.rfc-editor.org/rfc/rfc10008.html)は、クエリ意味論をリクエスト本文で運ぶ、安全で冪等なHTTP `QUERY`メソッドを定義します。

リクエストのコンテンツタイプがクエリ構文を識別します。

サーバーは、`Content-Type`がない場合や一貫しない場合に勝手に解釈を変えてはいけません。

RFCが定義する失敗境界は次のとおりです。

| 条件 | ステータス境界 |
| --- | --- |
| 必須のクエリメディアタイプがない | `400 Bad Request` |
| クエリメディアタイプをサポートしない | `415 Unsupported Media Type` |
| メディアタイプは理解できるがクエリ本文が不正 | `422 Unprocessable Content` |
| 受け入れ可能な応答表現がない | `406 Not Acceptable` |

`Accept-Query`はサポートするクエリメディアタイプを応答で通知します。

これは構造化フィールドのリストであり、構造化されていないカンマ区切り文字列ではありません。

RFC 10008は保存済みクエリとクエリ結果に対する任意の`Location`と`Content-Location`の用途も定義しますが、txBASEはそれらのリソースを現在作成せず、フィールドも返しません。

txBASEは`Accept-Query: "application/json"`を通知し、JSONクエリ文書だけを受け付けます。

クエリ本文はリクエスト意味論の一部です。

将来のキャッシュは本文を無視してURIだけをキーにしてはいけません。

現在のサーバーは、成功したQUERY応答に対する1つのバイト範囲をサポートします。

サポートする単一バイト範囲には`206 Partial Content`または`416 Range Not Satisfiable`を返し、未サポートまたは複数の範囲は無視します。

範囲処理は表現転送の機能であり、QUERYのメソッド意味論を変えません。

クエリに`page_size`がある場合、成功したJSON表現は`records`とnull許容の`cursor`メンバーを持つオブジェクトです。

`sort`がなければ、発行するカーソルは物理DBFレコード位置とテーブル表現のスナップショットタグを含むバージョン付きトークンです。

`sort`があれば、同じソートフィールド、方向、スナップショットタグに結び付いたバージョン付きキーセットトークンです。

テーブル変更後に発行済みカーソルを再利用すると、異なる表現のページを混在させず`422`を返します。

互換性のためタグなしカーソルは引き続き受け付けます。

どちらの形式も`skip`と併用できません。

`QUERY /records/stream`と`QUERY /{table}/records/stream`は、pull型クエリルートと同じJSONリクエスト文書を使います。

ただし、許可する制御は`filter`、`projection`、`skip`、`limit`だけです。

成功した応答のメディアタイプは`application/x-ndjson`です。

ラッパー配列またはcursorを使わず、コンパクトなJSONレコードを一行ずつ返します。

サーバーは`Content-Length`を省略するため、HTTP/1.1では固定容量チャネルを使う有界スナップショット生成側からchunked transferで返します。

不正な入力はストリーミング開始前に既存の`400`、`415`、`422`境界で拒否します。

ストリームにはETag、バイト範囲、resume tokenの契約がありません。

応答ヘッダー送信後の評価に失敗した場合は接続を終了し、クライアントはクエリ全体を再試行しなければなりません。

## 5. CDC読み取りルート

`GET /cdc`は、テーブルまたはカタログを変更せずにコミット済みの変更イベントを読み取ります。

単一テーブルサーバーは、テーブルサイドカーにある`TXCD`イベントを返します。

カタログサーバーは、1ページの中で複数テーブルトランザクションの原子性を保つため、カタログサイドカーにある`TXCC`イベントを返します。

両方のルートは、任意の排他的なトランザクションIDカーソル`after`と、1から1,000までの`limit`を受け付けます。

既定のlimitは100です。

応答は`{"events": [...], "next_after": 12}`です。

後続イベントがなければ`next_after`は`null`になり、クライアントはその値を次の`after`カーソルとして渡せます。

値が0または数値でないカーソル、不正なlimit、重複したパラメーター、未知のパラメーターには`400`を返します。

このカーソルは確認応答、リース、永続的な利用者位置ではありません。

`HEAD /cdc`は、本文を返さず、`GET /cdc`と同じステータスと表現ヘッダーを返します。

カタログサーバーは、独立した名前付きテーブルの`TXCD`イベントをこのルートで公開しません。
これらのイベントは1つのカタログトランザクションを表さないためです。

## 6. 永続化と再試行

HTTPの冪等性はDBFの書き込み経路をクラッシュ安全にはしません。

現在の更新経路は、状態ペイロードの前に永続的な`TXOP`意図を記録し、WALを同期してからDBFまたはmemoサイドカーを置き換えます。

起動時の復旧は、状態ペイロードがない場合にサポート対象の意図を再実行します。

`POST /transaction`はすべての操作を非公開のテーブルコピーへ適用し、1つのスナップショットまたはWALコミットを永続化します。

この経路は`DbfTransaction`を通ります。

検証または操作が失敗した場合はコピーを破棄し、現在のDBFを変更しません。

コミットIDはそのDBFに対して永続化され、再起動またはWAL復旧後も継続します。

現在の境界は1つのDBFテーブルであり、テーブル間アトミック性、カタログ全体のトランザクションID、MVCC可視性は提供しません。

Rust APIの境界は、[スナップショットトランザクション](transactions.md)で定義します。

カタログサーバーの`POST /transaction`は`/table/records`と`/table/records/{id}`の更新パスを受け付けます。

すべての対象テーブルを1つのカタログロック下で準備し、DBFと変更サイドカーをディレクトリジャーナルでコミットし、次のカタログ読み取り時に未完了の準備をロールバックします。

テーブル間のアトミックコミットとクラッシュ復旧を提供し、JSONと`X-Txbase-Transaction-Id`で永続的なカタログジャーナルトランザクションIDを返します。

このIDは順序と識別の境界であり、MVCC可視性ではありません。

テーブルロックは保存経路を直列化し、独立してロードされた古いテーブルは拒否します。

自動ネットワーク再試行、リクエスト重複排除、複数writerのマージは実装していません。

成功したTCP交換だけからexactly-onceの効果を推測してはいけません。

## 6. 今後の HTTP 作業

実装前に次の項目には明示的な契約が必要です。

- QUERY本文に対する`Content-Location`とキャッシュキーの規則。
- 長寿命クエリストリーム向けのランタイム固有非同期トレイト。
- CORSと認証方針。

## 主な参照先

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)
