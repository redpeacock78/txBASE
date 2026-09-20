# クエリ計画と外部語彙

この文書は、外部のクエリ用語とローカルプランナーの契約を分離します。

MongoDB は語彙とトレードオフの参照先です。

txBASE は MongoDB のクエリ互換性やプランナー互換性を主張しません。

## 1. 外部クエリ語彙

[MongoDB のクエリ述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)は、述語を比較、論理、配列、要素、評価、ビット演算、地理空間、その他の分類に分けています。

現在の txBASE のサブセットは、比較、所属、論理、有界なフィールド式までです。

MongoDB の [find コマンド](https://www.mongodb.com/docs/manual/reference/command/find/)は、フィルターとプロジェクション、ソート、スキップ、リミット、ヒント、その他のカーソル制御を分けています。

この分離により、txBASE はクエリ検証、結果整形、プランナーのアクセス経路を独立して変更できます。

MongoDB の find コマンドは初期バッチとカーソル識別子を返します。

Firestore の[クエリカーソルの説明](https://firebase.google.com/docs/firestore/query-data/query-cursors)は、一つのバッチの最後の文書を次のバッチの開始点に使います。

txBASE は物理ページとソートページでこの境界の考え方を採用します。

サーバー側のカーソル寿命やスナップショット分離は主張しません。

MongoDB の[比較述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)は `$eq`、`$gt`、`$gte`、`$lt`、`$lte`、`$ne`、`$in`、`$nin` などを定義しています。

MongoDB の [BSON 比較順序](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)と [`$gt` の型ブラケット規則](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)は別の境界を定義します。

MongoDB は BSON 固有の型規則と配列規則で BSON 値を比較します。

txBASE は独自の明示的なスカラー規則でデコード済み JSON 値を比較します。

したがって、同じ演算子名でも混在型、欠損フィールド、配列、文書に対する結果が同じとは限りません。

MongoDB の[論理述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)は `$and`、`$or`、`$nor`、`$not` を定義しています。

MongoDB の [`$expr` 述語](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)は、同じ文書内の二つのフィールドを比較する式を含め、述語内の式を許可します。

txBASE は有界な式木を実装します。

`{"$gt":["$LEFT","$RIGHT"]}` のような比較リーフを `$and`、`$or`、`$not` で合成できます。

各比較は二つのスカラーまたはフィールド参照オペランドだけを持ちます。

`$` で始まる文字列オペランドはドット区切りフィールド参照です。

それ以外のスカラーオペランドはリテラルです。

どちらかのフィールド参照が欠損している場合、式は一致しません。

フィールド間比較は定数境界のインデックス検索ではないため、式の経路はテーブルスキャンを使います。

算術、正規表現、配列、文書の式は未サポートです。

MongoDB の[配列述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)には、txBASE が実装していない `$all`、`$elemMatch`、`$size` などがあります。

MongoDB には正規表現や式評価を含む広い[その他の述語分類](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/misc/)もあります。

これらをファイルネイティブな DBF クエリエンジンに追加するには、エンコード、資源上限、エラー契約を先に定義する必要があります。

## 2. プランナーの境界

[MongoDB のクエリ最適化ガイド](https://www.mongodb.com/docs/manual/core/query-optimization/)は、述語の選択性とインデックスキーの順序が調査するデータ量に影響する理由を説明します。

選択性の低い `$ne` や `$nin` は、選択性の高い等値述語と同じようにはインデックスの恩恵を受けないことも説明しています。

txBASE はレコードスキャンをクエリ実行器の参照経路として保ちます。

パス対応のクエリエントリポイントは、同じフィルター、ソート、プロジェクション、スキップ、リミットのパイプラインを適用する前に、外部スカラーキーの等値、範囲、順序付き走査を試します。

プランナーは `explain_query_at` を通じて `TableScan`、`EqualityIndex`、`RangeIndex`、`OrderedIndex`、`OrderedIndexPrefix`、`CompoundOrderedIndex` を報告します。

単一テーブル HTTP サーバーは `QUERY /explain` で同じ説明を公開します。

応答は `{"plan": {"kind": "table_scan"}}` または選択した名前、フィールド、方向を持つタグ付きインデックス計画です。

この説明は記述的なもので、速度向上を保証しません。

欠損、古い、壊れた、意味上サポートされないサイドカーは、任意の高速化構造であるため `TableScan` にフォールバックします。

インデックスをクエリ実行へ接続するには、パーサー変更だけでは足りません。

キーのエンコード、null と欠損フィールドの規則、重複順序、更新時の保守、復旧レコード、古いインデックスの検出、プランナー方針が必要です。

現在のプランナーは、トップレベルの直接等値、片側ごとに一つの境界を持つ範囲述語、単一フィールドの順序付き走査、正確な等値プレフィックスの後でインデックスサフィックスに一致する複合ソート要求を扱います。

有効な単一フィールド等値インデックスが複数ある場合、通常のフィルターパイプラインの前にレコード番号で積集合を取れます。

プランナーはサイドカーのアクティブレコード数と各単一フィールドインデックスの異なるキー数を使い、候補リストを読み込む前に等値のカーディナリティを推定します。

正確な候補リストを推定選択性の順で処理します。

これにより、述語ごとに期待カーディナリティが異なる場合の重複した所属確認を抑えます。

等値推定は一様分布を仮定します。

単一フィールドの範囲インデックスは永続化された等深度ヒストグラムを使い、重なるバケットのレコード数を合計します。

これらの統計は、有界な整数コスト推定に使います。

プランナーは、テーブルスキャンのアクティブレコード数とインデックス経路の正確な候補数を比較し、経路が要求された順序を完全には提供しない場合にメモリ上のソート作業を加えます。

これはローカルな計画ロジックであり、MongoDB プランナー互換性ではありません。

複数キーソートでは、単一フィールドインデックスが最初のキーの順序を提供します。

実行器は最初のキーが等しいグループ内で残りのキーをメモリ上でソートします。

昇順または混在方向の複合インデックスは、要求されたソートフィールドが正確な等値プレフィックスの後でインデックスフィールドに一致するとき、完全な順序を提供します。

プランナーはインデックス順とその完全な逆順を受け付けます。

そのため対応する混在方向の要求でもメモリ上のソートを避けられます。

等値、範囲、順序付きアクセス経路が同時に有効な場合、プランナーはテーブルスキャンと有効な各インデックス経路の有界な作業量を比較します。

テーブルスキャンの推定値にはアクティブレコード数を使います。

インデックス経路の推定値には正確な候補数を使い、経路が要求された順序の一部を実行器に残す場合はソート作業を加えます。

要求された順序を完全に提供する順序付き経路にはソート項を加えません。

候補数が同じ場合、複合定義は最短の定義、安定した名前の順で選びます。

コストが完全に同じ場合は既存の候補順を保つため、インデックス経路との同点ではテーブルスキャンを選びます。

この有界モデルは、インデックス I/O、メモリ、キャッシュ状態、照合、複合キーの範囲選択性を推定しません。

MongoDB の現在の指針は、複数フィールドを繰り返し検索する場合に複合インデックスを推奨しています。

txBASE の積集合はローカルな候補削減機能です。

MongoDB のプランナー互換性や、将来の複合インデックス契約を置き換えるものではありません。

MongoDB の[複合インデックスのソート順に関する説明](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)と[等値、ソート、範囲の指針](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)は、将来の複合インデックスプランナーがフィールド順を定義すべき理由を示します。

txBASE は現在、アクティブレコード数、等値用の一様な異なるキー数推定、単一フィールドの範囲ヒストグラム、複合定義ごとの方向メタデータ、スキャンと残りのソート作業に対する有界なコスト推定を持ちます。

I/O を考慮した完全なコストモデルは持ちません。

等値積集合は有界な候補前処理であり、カバードクエリやエンドツーエンドの高速化を主張するものではありません。

ロードマップでは、クエリ文書が実装戦略を暗黙に指定しないよう、インデックス設計をクエリ構文から分離しています。

## 3. 今後のクエリ作業

次の項目には個別の公開契約が必要です。

1. 欠損、null、照合、複合範囲の規則を明示した完全な式評価とコストベースのインデックス選択。
2. メモリ動作を有界にした追加の集約ステージとアキュムレータ。
3. プランナーが選択する結合戦略と、より広い結合意味論。
4. 長寿命ストリーム向けのランタイム固有非同期トレイト。
5. 小さな参照評価器との微分テスト。

これらの契約ができるまでは、レコードスキャンを単純な参照実行モデルとして保ちます。

## 主な参照先

- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB comparison predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)
- [MongoDB logical predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
- [MongoDB `$expr` predicate](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [MongoDB array predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)
- [MongoDB query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [MongoDB BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [MongoDB `$gt` type bracketing](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)
- [MongoDB compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [MongoDB equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [SQLite query planning](https://www.sqlite.org/queryplanner.html)
