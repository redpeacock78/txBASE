# 式モデル

この文書では、共有するスカラー式と論理式の部分集合を定義します。
記載するのはtxBASEの動作であり、MongoDB互換性ではありません。

## 1. 評価する箇所

`$expr`は、現在のレコードに対する論理式を受け付けます。

共有スカラー評価器を集約の`$set`と`$addFields`、グループキー、`$sortByCount`、`$bucket`と`$bucketAuto`のグループ式、スカラー値を取るアキュムレータ入力で使います。
数値アキュムレータでは、後述する有界な数値式の文法を使います。

`$minN`、`$maxN`、`$firstN`、`$lastN`の`n`は、グループの`_id`とリテラルだけに依存できます。
スコープ内の変数も再帰的に検査します。`$$ROOT`と`$$CURRENT`は、このグループキー制約を満たしません。

評価器はJSONスカラー値、`"$PRICE"`のようなフィールドパス、式の配列、演算子を1つだけ持つオブジェクトを受け付けます。
値を式として解釈させない場合は`$literal`を使います。
複数フィールドを持つ計算オブジェクトは未対応です。

## 2. 論理式

論理式では`$and`、`$or`、`$not`と、2オペランドの比較演算子`$eq`、`$ne`、`$gt`、`$gte`、`$lt`、`$lte`を使えます。
各演算子オブジェクトには演算子を1つだけ指定します。

`$and`と`$or`は左から評価し、結果が確定した時点で残りを評価しません。
比較オペランドに欠損値がある場合は`false`を返します。大小を比較できない値の組み合わせも`false`になります。

`$cond.if`と`$filter.cond`には、この論理式を指定します。
真偽値として扱う任意のスカラー式は受け付けません。

## 3. スカラー式

共有スカラー評価器は次の式を受け付けます。

- `$literal`と配列構築子。欠損した配列要素の値は`null`。
- オペランドを2つ持つ`$ifNull`。
- 3要素の配列形式、または`if`、`then`、`else`を持つオブジェクト形式の`$cond`。
- 2つ以上の文字列式を受け取る`$concat`、単項演算子の`$toLower`と`$toUpper`。
- 単項の数値演算子`$abs`、`$ceil`、`$floor`。
- 二項の数値演算子`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`。
- 後述する`$let`、`$map`、`$filter`、`$reduce`。

数値演算子は、数値リテラル、フィールド参照、変数参照、ネストした数値式を受け付けます。
表現可能な整数の結果はJSON整数のまま返します。それ以外の結果は有限なJSON数値でなければなりません。
0除算、0剰余、整数オーバーフロー、有限でない結果はエラーです。

欠損オペランドや型が合わないオペランドは、`$expr`では不一致になります。
`$set`と`$addFields`では、欠損値と型不一致のスカラー結果を`null`にします。
同じ集約ステージの式はステージ入力時点のレコードを参照するため、そのステージ内で計算した兄弟フィールドは参照できません。

## 4. 変数とレキシカルスコープ

変数は`"$$item.price"`のようにドル記号を2つ付けて参照します。
ドット区切りの接尾辞は、参照先の値に対するパスとして解決します。

ユーザー変数名はASCII小文字または非ASCII文字で始めます。
以降の文字にはASCII英字、数字、アンダースコア、非ASCII文字を使えます。
大文字と小文字は区別します。

利用できるシステム変数は`$$ROOT`、`$$CURRENT`、`$$IDX`です。
`$$ROOT`と`$$CURRENT`は、現在の評価境界に渡されたレコードを指します。
`arrayIndexAs`を省略した配列演算子では、配列要素を評価するときに`$$IDX`へ0始まりの添字を束縛します。
それ以外のシステム変数は拒否します。

`$let`には`vars`と`in`だけを指定します。
各束縛は外側のスコープで評価するため、同じ`vars`オブジェクト内の変数同士は参照できません。
束縛が有効なのは`in`の中だけです。内側のスコープでは外側の変数をシャドウできます。

```json
{
  "$let": {
    "vars": {"taxed": {"$multiply": ["$PRICE", 1.1]}},
    "in": {"$add": ["$$taxed", "$SHIPPING"]}
  }
}
```

## 5. 配列式

`$map`は各要素に`in`を適用し、入力順に結果を返します。
`$filter`は条件に一致した入力要素を入力順に返します。
`$reduce`は左から右へ畳み込み、1つの値を返します。

```json
{
  "$map": {
    "input": "$ITEMS",
    "as": "item",
    "arrayIndexAs": "position",
    "in": ["$$item.price", "$$position"]
  }
}
```

3つの演算子で`as`を省略すると`this`を使います。
`arrayIndexAs`には0始まりの要素番号を表す変数名を指定します。省略時は`$$IDX`を使います。
`$reduce`では`valueAs`も指定でき、省略時は`value`を使います。
1つの演算子内で、要素、累積値、独自の添字に別々の変数名を指定します。

`$filter.limit`は省略できます。
式の結果には正のJSON整数または`null`を指定します。`null`は上限なしを意味します。
欠損した結果やその他の値はエラーです。
一致する要素は指定数を超えて返しません。

3つの演算子は、入力が欠損または`null`なら`null`を返し、入力が配列でも`null`でもなければエラーにします。
空の配列を`$reduce`に渡すと`initialValue`を返します。
未束縛変数や不正なオプションはエラーです。

## 6. 評価上限と参照資料

1回のスカラー評価で走査できる配列要素は、ネストした配列式を合計して100,000件までです。
具体化する配列は、エンコード後のJSONで1 MiBまでです。
計算する文字列も1 MiBまでです。
上限を超えるとクエリエラーになります。

未対応の式演算子とシステム変数は拒否します。
JSONモデル、欠損値の規則、型順序はMongoDBと異なります。下記の資料は一部の演算子形式と境界条件を調べるためだけに参照しています。

## 関連文書

- [クエリモデル](query-model.md)
- [集約モデル](aggregation.md)
- [集約アキュムレータ](aggregation-accumulators.md)
- [品質契約マトリクス](quality-matrix.md)

## 主な参照先

- [MongoDBの`$expr`クエリ演算子](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [MongoDBの集約式](https://www.mongodb.com/docs/manual/reference/aggregation/)
- [MongoDBの集約式演算子](https://www.mongodb.com/docs/manual/reference/operator/aggregation/)
- [MongoDBの`$let`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/let/)
- [MongoDBの集約式変数](https://www.mongodb.com/docs/manual/reference/aggregation-variables/)
- [MongoDBの`$map`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/map/)
- [MongoDBの`$filter`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/filter/)
- [MongoDBの`$reduce`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/reduce/)
- [MongoDBの`$cond`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/cond/)
- [MongoDBの`$ifNull`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ifnull/)
- [MongoDBの`$literal`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/literal/)
- [MongoDBの`$concat`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/concat/)
- [MongoDBの`$toLower`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/tolower/)
- [MongoDBの`$toUpper`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/toupper/)
- [MongoDBの`$abs`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/abs/)
- [MongoDBの`$ceil`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ceil/)
- [MongoDBの`$floor`式](https://www.mongodb.com/docs/manual/reference/operator/aggregation/floor/)
