# 集約モデル

txBASEは、フィルターに使う同じJSONクエリ文書上の有界パイプラインとして集約を扱います。

集約の境界は、通常のカーソルページングおよび関係結合の境界とは分離しています。

## 1. 有界集約

クエリ文書には、`filter`と0個以上の先行`$match`ステージの後に、終端`$count`、終端`$distinct`、またはブロッキングな`$group`を1つ置けます。

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

入力部分は、0個以上の`$match`と`$unwind`、入力用の`$project`、`$sort`、`$skip`、`$limit`をそれぞれ最大1つ受け付け、その後に終端`$count`、終端`$distinct`、または`$group`を1つ受け付けます。

入力ステージはパイプラインに記載した順序で実行します。

`$unwind`は`{"$unwind": "$FIELD"}`の形式を使い、トップレベルのフィールド参照だけを受け付けます。

複数の`$unwind`は記載順で順番に適用します。

`$unwind`の後に置いた入力用の`$match`は、展開後のレコードをフィルターします。

入力用の`$sort`は、終端ステージの前に既存のJSONソート順と物理レコード順による安定した同値順を適用します。

入力用の`$skip`と`$limit`は、終端ステージの前にレコードを破棄または切り詰めます。

入力用の`$project`はクエリのプロジェクション規則を再利用し、入力フェーズで1回だけ、他の入力ステージと記載順に組み合わせて置けます。

入力形式が受け付ける包含または除外の値は`0`と`1`だけであり、計算プロジェクション式、空の指定、包含と除外の混在は未サポートです。

プロジェクションは次のステージの前に具体化されるため、後続の`$match`、`$group`、`$count`、`$distinct`はプロジェクション後のフィールドだけを参照します。

配列フィールドに対する`$unwind`は、入力配列の順序で要素ごとに入力レコードのコピーを1つ出力し、対象フィールドを要素で置き換えます。

存在しないフィールド、明示的な`null`、空配列はレコードを出力しません。

`null`でない配列以外のフィールドは、1要素配列へ変換せず集約を拒否します。

すべての`$unwind`が出力するレコード数の合計は、`$count`、`$distinct`、または`$group`の前に10,000件までに制限します。

ドット区切りのフィールドパス、`preserveNullAndEmptyArrays`、`includeArrayIndex`、その他の拡張された`$unwind`形式は未サポートです。

グループ出力には0個以上の`$match`を置けます。
その後に任意の`$project`を1つ、最後の`$sort`、`$skip`、`$limit`をそれぞれ最大1つ置けます。

`_id`は`null`または1つのドット区切りフィールド参照です。

サポートするアキュムレータは`$count: {}`、数値の`$sum`、`$min: "$FIELD"`、`$max: "$FIELD"`、`$first: "$FIELD"`、`$last: "$FIELD"`、`$push: "$FIELD"`、`$addToSet: "$FIELD"`、および有限なJSON数値に対する数値の`$avg`です。

数値の`$sum`と`$avg`のオペランドは、フィールド参照、数値リテラル、単項の`$abs`、または二項の`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`式を受け付けます。

有界な数値式の評価器は`$expr`と共有し、解決した値が欠損または数値以外の場合は`$sum`と`$avg`で無視します。

フィルターはグループ化より前に実行します。

結果は`_id`と名前付きアキュムレータフィールドを持つJSONオブジェクトの配列です。

`$project`はクエリのプロジェクション規則を再利用してグループ出力のフィールドを包含または除外します。

`$project`は`$group`の後、`$sort`、`$skip`、または`$limit`の前に置く必要があります。

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

欠損と`null`の`$min`および`$max`入力は無視します。

すべてが欠損または`null`のグループでは、そのアキュムレータは`null`になります。

非`null`の`$min`および`$max`値は、既存のJSON順序規則で比較できなければなりません。

比較できない値は拒否します。

`$first`と`$last`は、各グループ内の入力物理レコード順を使います。

明示的な`null`を含めて最初または最後のフィールド値を返し、欠損フィールドは`null`として返します。

`$push`は入力物理レコード順のすべてのフィールド値を返し、`$addToSet`はJSON値ごとに一度だけ初出順で返します。

どちらのアキュムレータも、存在しないフィールドを`null`として追加します。

すべての`$push`と`$addToSet`がマテリアライズする値の合計は10,000件に制限されます。

10,000を超えるグループは拒否し、集約とトップレベルの`sort`、`projection`、`skip`、`limit`、cursorページングの併用も拒否します。

`$sort`がない場合のグループ出力順は契約に含めませんが、現在の実装は決定的なキー順で出力します。

`$sort`は既存のJSONソート順と安定した同値順を使います。

`$limit`は0以上の整数を受け付け、ソート後の具体化されたグループ結果を切り詰めます。

`$skip`は0以上の整数を受け付け、ソート後かつ`$limit`の前に、具体化されたグループ結果を指定件数だけ破棄します。

`$skip`は`$group`の後、任意の`$project`または`$sort`の後、`$limit`の前に置く必要があります。

`$match`ステージはトップレベルの`filter`と同じ述語規則を使います。

入力に対する`$match`ステージは`$group`、`$count`、`$distinct`より前に置く必要があります。

グループ出力に対する`$match`ステージは`$group`の後、`$project`、`$sort`、`$skip`、`$limit`より前に置く必要があります。

`$count`は名前付きの0以上の整数フィールドを1つ持つ文書を返し、一致するレコードがない場合も0を返します。

`$distinct`は1つのフィールド参照を受け取り、重複しないフィールド値をJSON配列で返します。

欠損フィールドと明示的な`null`は1つの`null`値として扱います。

両ステージは終端であり、グループ出力ステージとは併用できません。

distinct出力は10,000値までです。

追加のgroup、count、distinctステージは未サポートです。

`$group`の後では、グループ出力用の`$limit`より後のステージは未サポートです。

`$expr`、`$sum`、`$avg`のオペランドは、クエリモデルで説明する有界な数値`$abs`、`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`の形式だけをサポートします。

より広い式評価は未サポートです。

MongoDBは`$group`をブロッキングステージとして説明し、[$count と $sum を含む集約ステージの仕様](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)を定義しています。

txBASEはMongoDBの完全なパイプライン互換性を主張せず、その別個の[`$count`ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)を有界なステージとして表現します。

## 関連文書

- [クエリモデル](query-model.md)
- [クエリ計画と外部語彙](query-planning.md)
- [品質契約マトリクス](quality-matrix.md)

## 主な参照先

- [MongoDB の`$group`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB の`$sum`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDB の`$avg`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDB の`$count`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
- [MongoDB の`$project`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/project/)
- [MongoDB の`$unwind`集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/unwind/)
