# 集約モデル

txBASEは、フィルターに使う同じJSONクエリ文書上の有界パイプラインとして集約を扱います。

集約の境界は、通常のカーソルページングおよび関係結合の境界とは分離しています。

## 1. 有界集約

クエリ文書には、`filter`と0個以上の先行`$match`ステージの後に、終端`$count`、終端`$distinct`、ブロッキングな`$group`、ブロッキングな`$bucket`、または`$sortByCount`を1つ置けます。

```json
{
  "filter": {"ACTIVE": true},
  "aggregate": [
    {
      "$group": {
        "_id": "$COUNTRY",
        "count": {"$count": {}},
        "total_age": {"$sum": "$AGE"}
      }
    }
  ]
}
```

入力部分は、0個以上の`$match`と`$unwind`、合計で最大1つの入力用`$set`または`$addFields`、入力用の`$project`、`$sort`、`$skip`、`$limit`をそれぞれ最大1つ受け付け、その後に終端`$count`、終端`$distinct`、`$group`、`$bucket`、または`$sortByCount`を1つ受け付けます。

入力ステージはパイプラインに記載した順序で実行します。

`$unwind`は`{"$unwind": "$FIELD"}`のようなトップレベルのフィールド参照、または`path`、`includeArrayIndex`、`preserveNullAndEmptyArrays`を持つドキュメントを受け付けます。

ドキュメント形式では`path`が必須であり、トップレベルの`includeArrayIndex`フィールド名と、任意の真偽値`preserveNullAndEmptyArrays`を指定できます。

`includeArrayIndex`は各配列要素に0から始まる整数を、保持された欠損、null、空配列の入力に`null`を書き込みます。

複数の`$unwind`は記載順で順番に適用します。

`$unwind`の後に置いた入力用の`$match`は、展開後のレコードをフィルターします。

入力用の`$sort`は、終端ステージの前に既存のJSONソート順と物理レコード順による安定した同値順を適用します。

入力用の`$skip`と`$limit`は、終端ステージの前にレコードを破棄または切り詰めます。

`$set`と`$addFields`は同じ意味を持つ別名であり、既存フィールドを保ったまま、後続ステージの前に名前付きトップレベルフィールドを計算する有界な入力ステージです。

計算フィールドは、スカラーリテラル、ドット区切りを含むフィールド参照、`$literal`、オペランドをちょうど2つ持つ`$ifNull`、または有界な数値`$abs`、`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`式を受け付けます。

同じステージのすべての式はステージ入力時点のレコードを参照するため、同じステージで計算したフィールドを別の計算フィールドから参照できません。

欠損フィールド参照と、欠損または数値以外になった数値式の結果は`null`になります。

`$ifNull`は欠損と明示的な`null`をnullとして扱い、その場合にフォールバックを評価します。

既存のトップレベルフィールドは上書きしますが、ドット区切りの出力フィールド名と`$literal`の外側にある配列リテラルは未サポートです。

入力用の`$project`はクエリのプロジェクション規則を再利用し、入力フェーズで1回だけ、他の入力ステージと記載順に組み合わせて置けます。

入力形式が受け付ける包含または除外の値は`0`と`1`だけであり、計算プロジェクション式、空の指定、包含と除外の混在は未サポートです。

プロジェクションは次のステージの前に具体化されるため、後続の`$match`、`$group`、`$bucket`、`$count`、`$distinct`はプロジェクション後のフィールドだけを参照します。

配列フィールドに対する`$unwind`は、入力配列の順序で要素ごとに入力レコードのコピーを1つ出力し、対象フィールドを要素で置き換えます。

デフォルトでは、存在しないフィールド、明示的な`null`、空配列はレコードを出力しません。

`preserveNullAndEmptyArrays: true`を指定すると、これらの入力はそれぞれ1レコードを出力します。
空配列のフィールドはそのレコードから削除し、明示的な`null`は`null`のまま、存在しないフィールドは存在しないままにします。

`null`でない配列以外のフィールドは、1要素配列へ変換せず集約を拒否します。

保持されたレコードを含むすべての`$unwind`出力レコード数の合計は、`$count`、`$distinct`、`$group`、または`$bucket`の前に10,000件までに制限します。

ドット区切りのフィールドパス、`path`と同じ`includeArrayIndex`名、その他の拡張された`$unwind`形式は未サポートです。

`$bucket`は`groupBy`、`boundaries`、任意の`default`値を使って、レコードを数値範囲へ分類します。

```json
{
  "$bucket": {
    "groupBy": "$AGE",
    "boundaries": [0, 20, 40],
    "default": "other",
    "output": {"count": {"$count": {}}}
  }
}
```

`groupBy`は1つのフィールド参照でなければならず、`boundaries`は2つ以上の有限なJSON数値を昇順で重複なく含まなければなりません。

各範囲は下端を含み、上端を含みません。

`groupBy`の値が欠損、null、数値以外、または範囲外の場合、`default`があればその値へ分類し、なければ集約を拒否します。

空でない範囲ごとに1つの文書を出力し、`_id`には範囲の下端を設定します。

値が存在するdefaultバケットも1つの文書を出力し、`_id`には`default`の値を設定します。

空バケットは出力せず、defaultバケットは範囲バケットの後に出力します。

`$sortByCount`はフィールド参照ごとにレコードをグループ化し、グループ値を`_id`、件数を`count`として、`count`の降順で出力します。

```json
{
  "$sortByCount": "$COUNTRY"
}
```

txBASEのサブセットが受け付けるのは1つのフィールド参照だけであり、任意の式とドキュメントリテラルは未サポートです。

欠損したフィールドと明示的な`null`は同じグループになります。

空のグループは出力せず、グループ数は10,000件までに制限し、ディスクへ退避しません。

組み込みの件数降順は、後続のグループ出力ステージより前に適用します。
後続の`$sort`で順序を上書きできます。

`output`を省略すると、`$bucket`は`count`アキュムレータを出力します。

`output`を指定した場合は、`_id`をバケットステージが設定する点を除き、`$group`と同じ有界なアキュムレータ形式を使います。

範囲は10,000個までであり、ディスクへ退避せず、`$push`と`$addToSet`に対する10,000値の具体化上限を共有します。

グループ、バケット、または`$sortByCount`の出力には0個以上の`$match`を置けます。
その後に任意の`$project`を1つ、最後の`$sort`、`$skip`、`$limit`をそれぞれ最大1つ置けます。

`_id`は`null`または1つのドット区切りフィールド参照です。

サポートするアキュムレータは`$count: {}`、数値の`$sum`、`$min: "$FIELD"`、`$max: "$FIELD"`、`$first: "$FIELD"`、`$last: "$FIELD"`、`$push: "$FIELD"`、`$addToSet: "$FIELD"`、および有限なJSON数値に対する数値の`$avg`、`$stdDevPop`、`$stdDevSamp`です。

数値の`$sum`と`$avg`のオペランドは、フィールド参照、数値リテラル、単項の`$abs`、または二項の`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`式を受け付けます。

有界な数値式の評価器は`$expr`と共有し、解決した値が欠損または数値以外の場合は`$sum`と`$avg`で無視します。

フィルターはグループ化またはバケット化より前に実行します。

結果は`$group`または`$bucket`が生成した`_id`と名前付きアキュムレータフィールド、または`$sortByCount`が生成した`_id`と`count`を持つJSONオブジェクトの配列です。

`$project`はクエリのプロジェクション規則を再利用してグループまたはバケット出力のフィールドを包含または除外します。

`$project`は`$group`、`$bucket`、または`$sortByCount`の後、`$sort`、`$skip`、または`$limit`の前に置く必要があります。

包含と除外は混在できません。

プロジェクションは後続の`$sort`より前に適用するため、除外したフィールドをソートすると既存の欠損値順になります。

欠損したグループフィールドは`null`になります。

そのため欠損値と明示的な`null`は同じグループになります。

欠損、`null`、数値以外の`$sum`入力は0として扱います。

数値の`$sum`リテラルは入力レコードごとに一度加算します。

`$sum`の数値入力がすべて整数なら、結果もJSONの整数になります。

小数を1つでも含む場合、`$sum`は有限なJSON浮動小数点数を返します。

`$sum`の累積結果が有限でない場合、またはJSONで表現できない場合は拒否します。

欠損、`null`、数値以外の`$avg`入力は無視します。

すべてが欠損または数値以外のグループは`null`を返し、非有限の累積結果は拒否します。

`$stdDevPop`は母標準偏差を返し、`$stdDevSamp`は標本標準偏差を返します。

どちらの標準偏差アキュムレータも、`$sum`および`$avg`と同じ有界な数値式を受け付けます。

標準偏差に対する欠損、`null`、数値以外の入力は無視します。

すべてが欠損または数値以外のグループでは、どちらのアキュムレータも`null`を返します。

数値入力が1つの場合、`$stdDevPop`は`0`を返し、`$stdDevSamp`は数値入力が2つになるまで`null`を返します。

どちらのアキュムレータもグループごとに一定量のメモリだけを使い、有限なJSON浮動小数点数を返し、中間値または結果が非有限の場合は拒否します。

欠損と`null`の`$min`および`$max`入力は無視します。

すべてが欠損または`null`のグループでは、そのアキュムレータは`null`になります。

非`null`の`$min`および`$max`値は、既存のJSON順序規則で比較できなければなりません。

比較できない値は拒否します。

`$first`と`$last`は、各グループ内の入力物理レコード順を使います。

明示的な`null`を含めて最初または最後のフィールド値を返し、欠損フィールドは`null`として返します。

`$push`は入力物理レコード順のすべてのフィールド値を返し、`$addToSet`はJSON値ごとに一度だけ初出順で返します。

どちらのアキュムレータも、存在しないフィールドを`null`として追加します。

すべての`$push`と`$addToSet`がマテリアライズする値の合計は10,000件に制限されます。

10,000を超えるグループ、`$sortByCount`のグループ、またはバケット範囲は拒否し、集約とトップレベルの`sort`、`projection`、`skip`、`limit`、cursorページングの併用も拒否します。

`$sort`がない場合のグループまたはバケット出力順は契約に含めませんが、現在の実装は決定的なキー順または境界順で出力します。

`$sort`は既存のJSONソート順と安定した同値順を使います。

`$sortByCount`の出力は、後続のグループ出力ステージより前に`count`の降順で並びます。

`$limit`は0以上の整数を受け付け、ソート後の具体化されたグループ結果を切り詰めます。

`$skip`は0以上の整数を受け付け、ソート後かつ`$limit`の前に、具体化されたグループ結果を指定件数だけ破棄します。

`$skip`は`$group`または`$bucket`の後、任意の`$project`または`$sort`の後、`$limit`の前に置く必要があります。

`$match`ステージはトップレベルの`filter`と同じ述語規則を使います。

入力に対する`$match`ステージは`$group`、`$bucket`、`$sortByCount`、`$count`、`$distinct`より前に置く必要があります。

グループ、バケット、または`$sortByCount`出力に対する`$match`ステージは`$group`、`$bucket`、または`$sortByCount`の後、`$project`、`$sort`、`$skip`、`$limit`より前に置く必要があります。

`$count`は名前付きの0以上の整数フィールドを1つ持つ文書を返し、一致するレコードがない場合も0を返します。

`$distinct`は1つのフィールド参照を受け取り、重複しないフィールド値をJSON配列で返します。

欠損フィールドと明示的な`null`は1つの`null`値として扱います。

両ステージは終端であり、グループ出力ステージとは併用できません。

distinct出力は10,000値までです。

追加のgroup、bucket、sort-by-count、count、distinctステージは未サポートです。

`$group`、`$bucket`、または`$sortByCount`の後では、グループ出力用の`$limit`より後のステージは未サポートです。

`$expr`、`$sum`、`$avg`、`$stdDevPop`、`$stdDevSamp`のオペランドは、クエリモデルで説明する有界な数値`$abs`、`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`の形式だけをサポートします。

より広い式評価は未サポートです。

MongoDBは`$group`をブロッキングステージとして説明し、[$count と $sum を含む集約ステージの仕様](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)を定義しています。

MongoDBは`$sortByCount`を、`$group`の後に`count`の降順ソートを続ける処理と同等のグループ化ステージとして説明しています。txBASEはこの動作を保ちつつ、グループ化式を1つのフィールド参照に制限します。

txBASEはMongoDBの完全なパイプライン互換性を主張せず、その別個の[`$count`ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)を有界なステージとして表現します。

## 関連文書

- [クエリモデル](query-model.md)
- [クエリ計画と外部語彙](query-planning.md)
- [品質契約マトリクス](quality-matrix.md)

## 主な参照先

- [MongoDB の`$group`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB の`$sum`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDB の`$avg`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDB の`$stdDevPop`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevpop/)
- [MongoDB の`$stdDevSamp`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevsamp/)
- [MongoDB の`$count`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
- [MongoDB の`$bucket`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/bucket/)
- [MongoDB の`$sortByCount`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sortByCount/)
- [MongoDB の`$project`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/project/)
- [MongoDB の`$set`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/set/)
- [MongoDB の`$addFields`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/addfields/)
- [MongoDB の`$ifNull`式演算子](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ifnull/)
- [MongoDB の`$literal`式演算子](https://www.mongodb.com/docs/manual/reference/operator/aggregation/literal/)
- [MongoDB の`$unwind`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/unwind/)
