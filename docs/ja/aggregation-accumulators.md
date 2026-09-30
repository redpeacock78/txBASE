# 集約アキュムレータ

`$group`、`$bucket`、`$bucketAuto`は同じ有界なアキュムレータを使います。パイプライン内の配置、ステージ順、バケットの動作は[集約モデル](aggregation.md)を参照してください。

## 数値アキュムレータ

- `$count: {}`はグループ内の各レコードを数える。
- `$sum`はフィールド参照、数値リテラル、`$abs`、二項の`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`式を受け付ける。欠損、`null`、数値以外の結果は加算対象外とする。入力がすべて整数ならJSONの整数を返し、小数を含む場合は有限な浮動小数点数を返す。
- `$avg`は`$sum`と同じ式を受け付ける。欠損、`null`、数値以外の結果を無視し、数値入力がないグループでは`null`を返す。
- `$stdDevPop`と`$stdDevSamp`は`$sum`と同じ式を受け付け、グループごとに一定量のメモリを使う。欠損、`null`、数値以外の結果を無視する。数値入力が1つの場合、`$stdDevPop`は0を返し、`$stdDevSamp`は2つ目の入力があるまで`null`を返す。

有限でない中間値または結果は拒否します。数値式評価器は`$expr`と共有します。

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

`n`は1以上10,000以下の整数に評価されなければなりません。定数、または出力グループの`_id`だけを参照する式を指定できます。グループを作成するときに一度評価します。この上限はtxBASE固有です。

txBASEは`$minN`を昇順、`$maxN`を降順で返し、同値は入力順に並べます。MongoDBはこれらのアキュムレータの出力順を規定していないため、決定的な順序はtxBASE独自の動作です。

比較順はJSONで表現できる値についてMongoDBのBSON型順に従います。順序は`null`、数値、文字列、オブジェクト、配列、真偽値です。文字列はバイナリ順で比較し、配列は辞書順で比較します。オブジェクトはtxBASEのJSONマップ反復順に、値の型、フィールド名、値を比較します。BSON固有の値とBSONオブジェクトの挿入順はJSONデータモデルの対象外です。

## 入力順と配列アキュムレータ

- `$first`と`$last`はグループ内の入力物理レコード順を使う。明示的な`null`を含めて選択したフィールド値を返し、欠損フィールドは`null`になる。
- `$push`は入力物理レコード順ですべてのフィールド値を返す。`$addToSet`は構造的に等しいJSON値を1つにまとめ、最初に現れた順で返す。どちらも欠損フィールドを`null`として追加する。

`$push`、`$addToSet`、`$minN`、`$maxN`が保持する値の合計は、集約結果ごとに10,000件までです。そのため`n`も10,000を超えられません。

## 主な参照先

- [MongoDBの`$group`ステージ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDBの`$sum`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDBの`$avg`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDBの`$stdDevPop`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevpop/)
- [MongoDBの`$stdDevSamp`アキュムレータ](https://www.mongodb.com/docs/manual/reference/operator/aggregation/stddevsamp/)
- [MongoDBの`$minN`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/minn/)
- [MongoDBの`$maxN`アキュムレータ](https://www.mongodb.com/docs/v8.0/reference/operator/aggregation/maxn/)
- [MongoDBのBSON比較順](https://www.mongodb.com/docs/v8.0/reference/bson-type-comparison-order/)
