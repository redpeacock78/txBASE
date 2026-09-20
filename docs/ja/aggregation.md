# 集約モデル

txBASE は、フィルターに使う同じ JSON クエリ文書上の有界パイプラインとして集約を扱います。

集約の境界は、通常のカーソルページングおよび関係結合の境界とは分離しています。

## 1. 有界集約

クエリ文書には、`filter` と 0 個以上の先行 `$match` ステージの後に、終端 `$count`、終端 `$distinct`、またはブロッキングな `$group` を一つ置けます。

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

現在の集約境界は、0 個以上の `$match` の後に終端 `$count`、終端 `$distinct`、または `$group` を一つだけ受け付けます。

グループ出力には 0 個以上の `$match` を置き、その後に任意の `$project` を一つ、最後の `$sort` を最大一つ、最後の `$limit` を最大一つ置けます。

`_id` は `null` または一つのドット区切りフィールド参照です。

サポートするアキュムレータは `$count: {}`、数値の `$sum: "$FIELD"`、`$min: "$FIELD"`、`$max: "$FIELD"`、`$first: "$FIELD"`、`$last: "$FIELD"`、`$push: "$FIELD"`、`$addToSet: "$FIELD"`、および有限な JSON 数値に対する `$avg: "$FIELD"` です。

フィルターはグループ化より前に実行します。

結果は `_id` と名前付きアキュムレータフィールドを持つ JSON オブジェクトの配列です。

`$project` はクエリのプロジェクション規則を再利用してグループ出力のフィールドを包含または除外します。

`$project` は `$group` の後、`$sort` または `$limit` の前に置く必要があります。

包含と除外は混在できません。

プロジェクションは後続の `$sort` より前に適用するため、除外したフィールドをソートすると既存の欠損値順になります。

欠損したグループフィールドは `null` になります。

そのため欠損値と明示的な `null` は同じグループになります。

欠損、`null`、数値以外の `$sum` 入力は 0 として扱います。

`$sum` の数値入力がすべて整数なら、結果も JSON の整数になります。

小数を一つでも含む場合、`$sum` は有限な JSON 浮動小数点数を返します。

`$sum` の累積結果が有限でない場合、または JSON で表現できない場合は拒否します。

欠損、`null`、数値以外の `$avg` 入力は無視します。

すべてが欠損または数値以外のグループは `null` を返し、非有限の累積結果は拒否します。

欠損と `null` の `$min` および `$max` 入力は無視します。

すべてが欠損または `null` のグループでは、そのアキュムレータは `null` になります。

非 `null` の `$min` および `$max` 値は、既存の JSON 順序規則で比較できなければなりません。

比較できない値は拒否します。

`$first` と `$last` は、各グループ内の入力物理レコード順を使います。

明示的な `null` を含めて最初または最後のフィールド値を返し、欠損フィールドは `null` として返します。

`$push` は入力物理レコード順のすべてのフィールド値を返し、`$addToSet` は JSON 値ごとに一度だけ初出順で返します。

どちらのアキュムレータも、存在しないフィールドを `null` として追加します。

すべての `$push` と `$addToSet` がマテリアライズする値の合計は 10,000 件に制限されます。

10,000 を超えるグループは拒否し、集約とトップレベルの `sort`、`projection`、`skip`、`limit`、cursor ページングの併用も拒否します。

`$sort` がない場合のグループ出力順は契約に含めませんが、現在の実装は決定的なキー順で出力します。

`$sort` は既存の JSON ソート順と安定した同値順を使います。

`$limit` は 0 以上の整数を受け付け、ソート後の具体化されたグループ結果を切り詰めます。

`$match` ステージはトップレベルの `filter` と同じ述語規則を使います。

入力に対する `$match` ステージは `$group`、`$count`、`$distinct` より前に置く必要があります。

グループ出力に対する `$match` ステージは `$group` の後、`$project`、`$sort`、`$limit` より前に置く必要があります。

`$count` は名前付きの 0 以上の整数フィールドを一つ持つ文書を返し、一致するレコードがない場合も 0 を返します。

`$distinct` は一つのフィールド参照を受け取り、重複しないフィールド値を JSON 配列で返します。

欠損フィールドと明示的な `null` は一つの `null` 値として扱います。

両ステージは終端であり、グループ出力ステージとは併用できません。

distinct 出力は 10,000 値までです。

`$limit` 後のステージ、追加の group、count、distinct ステージは未サポートです。

`$expr` のオペランドは、クエリモデルで説明する有界な数値 `$abs`、`$add`、`$subtract`、`$multiply`、`$divide`、`$mod` の形式だけをサポートします。

より広い式評価は未サポートです。

MongoDB は `$group` をブロッキングステージとして説明し、[$count と $sum を含む集約ステージの仕様](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)を定義しています。

txBASE は MongoDB の完全なパイプライン互換性を主張せず、その別個の [`$count` ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)を有界なステージとして表現します。

## 関連文書

- [クエリモデル](query-model.md)
- [クエリ計画と外部語彙](query-planning.md)
- [品質契約マトリクス](quality-matrix.md)

## 主な参照先

- [MongoDB の `$group` 集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB の `$count` 集約ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
