# 集約アキュムレータ

`$group`、`$bucket`、`$bucketAuto`は同じ有界なアキュムレータを使います。パイプライン内の配置、ステージ順、バケットの動作は[集約モデル](aggregation.md)を参照してください。

## 数値アキュムレータ

- `$count: {}`はグループ内の各レコードを数える。
- `$sum`はフィールド参照、数値リテラル、`$abs`、`$ceil`、`$floor`、二項の`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`式を受け付ける。欠損、`null`、数値以外の結果は加算対象外とする。入力がすべて整数ならJSONの整数を返し、小数を含む場合は有限な浮動小数点数を返す。
- `$avg`は`$sum`と同じ式を受け付ける。欠損、`null`、数値以外の結果を無視し、数値入力がないグループでは`null`を返す。
- `$stdDevPop`と`$stdDevSamp`は`$sum`と同じ式を受け付け、グループごとに一定量のメモリを使う。欠損、`null`、数値以外の結果を無視する。数値入力が1つの場合、`$stdDevPop`は0を返し、`$stdDevSamp`は2つ目の入力があるまで`null`を返す。

有限でない中間値または結果は拒否します。数値式評価器は`$expr`と共有します。

`$ceil`と`$floor`は、それぞれ数値式の数学的な切り上げと切り下げを返します。
結果が`i64`または`u64`の範囲に収まる場合はJSON整数で表します。

## 極値とN件選択

`$min`と`$max`はフィールド参照を受け付けます。欠損と`null`を無視し、それ以外の値がないグループでは`null`を返します。

`$minN`と`$maxN`は`input`式と`n`式を受け付けます。

```json
{
  "$minN": {
    "input": ["$SCORE", "$PLAYER"],
    "n": 2
  }
}
```

`input`はレコードごとに評価します。結果が欠損または`null`の場合は無視し、重複値は出力に残します。配列式はフィールド参照と、対応するスカラー式を組み合わせられます。

すべての結果が欠損または`null`の場合、空配列を返します。

`$firstN`と`$lastN`も同じ`input`式と`n`式を受け付けます。

```json
{
  "first": {"$firstN": {"input": "$SCORE", "n": 2}},
  "last": {"$lastN": {"input": "$SCORE", "n": 2}}
}
```

どちらもレコードごとに`input`を評価し、グループの入力順で値を保持します。欠損した入力は`null`になり、明示的な`null`と重複値も結果に残ります。`$firstN`は先頭の`n`件を返し、`$lastN`は末尾の`n`件を入力順のまま返します。グループのレコード数が`n`未満なら、存在する値をすべて返します。

前段に入力`$sort`がなければ、グループの入力順は物理レコード順です。入力`$sort`があれば、その順序を使います。

txBASEがサポートする`$firstN`と`$lastN`は、`$group`、`$bucket`、`$bucketAuto`のアキュムレータ形式です。MongoDBの配列演算子の形式とwindow operator形式は対象外です。

`n`は1以上10,000以下の整数に評価されなければなりません。定数、または出力グループの`_id`だけを参照する式を指定できます。グループを作成するときに一度評価します。この上限はtxBASE固有です。

txBASEは`$minN`を昇順、`$maxN`を降順で返し、同値は入力順に並べます。MongoDBはこれらのアキュムレータの出力順を規定していないため、決定的な順序はtxBASE独自の動作です。

比較順はJSONで表現できる値についてMongoDBのBSON型順に従います。順序は`null`、数値、文字列、オブジェクト、配列、真偽値です。文字列はバイナリ順で比較し、配列は辞書順で比較します。オブジェクトはtxBASEのJSONマップ反復順に、値の型、フィールド名、値を比較します。BSON固有の値とBSONオブジェクトの挿入順はJSONデータモデルの対象外です。

## 入力順と配列アキュムレータ

- `$first`と`$last`はグループの入力順を使い、前段に入力`$sort`がなければ物理レコード順、あればその順序になる。選択したフィールド値を返し、明示的な`null`を保持する。欠損フィールドは`null`になる。
- `$push`はグループの入力順ですべてのフィールド値を返す。`$addToSet`は構造的に等しいJSON値を1つにまとめ、最初に現れた順を保つ。どちらも欠損フィールドを`null`として追加する。
- `$mergeObjects`は、ドキュメントに評価される対応済みのスカラー式を受け付ける。欠損と`null`の結果を無視し、グループの入力順にフィールドをマージする。同じフィールド名は後のドキュメントの値で上書きする。`null`以外の非ドキュメント値はエラーにする。結果がすべて欠損または`null`なら`{}`を返す。ネストしたオブジェクトは再帰的にマージしない。

txBASEは`$mergeObjects`を`$group`、`$bucket`、`$bucketAuto`のアキュムレータ形式だけでサポートし、式形式は対象外とします。

`$push`、`$addToSet`、`$minN`、`$maxN`、`$firstN`、`$lastN`が保持する値の合計は、各`$group`、`$bucket`、`$bucketAuto`ステージの実行につき10,000件までです。この上限は、そのステージの全グループと全アキュムレータで共有します。
`$mergeObjects`では、グループの結果に追加したフィールドごとに同じ上限へ数えます。そのグループですでに存在するフィールドを上書きしても再計上しません。
各`n`にも個別に10,000の上限を適用しますが、ステージ共通の上限に先に達することがあります。

## 主な参照先

- [MongoDBの`$group`ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDBの`$sum`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDBの`$avg`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDBの`$stdDevPop`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevpop/)
- [MongoDBの`$stdDevSamp`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevsamp/)
- [MongoDBの`$ceil`式演算子](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ceil/)
- [MongoDBの`$floor`式演算子](https://www.mongodb.com/docs/manual/reference/operator/aggregation/floor/)
- [MongoDBの`$minN`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/minn/)
- [MongoDBの`$maxN`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/maxn/)
- [MongoDBの`$firstN`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/firstn/)
- [MongoDBの`$lastN`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/lastn/)
- [MongoDB 8.0の`$mergeObjects`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/mergeObjects/)
- [MongoDBのBSON比較順](https://www.mongodb.com/docs/v8.0/reference/bson-type-comparison-order/)
