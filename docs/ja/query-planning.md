# クエリ計画と外部語彙

この文書は、外部のクエリ用語とローカルプランナーの契約を分離します。

MongoDBは語彙とトレードオフの参照先です。

txBASEはMongoDBのクエリ互換性やプランナー互換性を主張しません。

## 1. 外部クエリ語彙

[MongoDB のクエリ述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)は、述語を比較、論理、配列、要素、評価、ビット演算、地理空間、その他の分類に分けています。

現在のtxBASEのサブセットは、比較、所属、論理、有界な配列、有界なフィールド式を含みます。

MongoDBの[find コマンド](https://www.mongodb.com/docs/manual/reference/command/find/)は、フィルターとプロジェクション、ソート、スキップ、リミット、ヒント、その他のカーソル制御を分けています。

この分離により、txBASEはクエリ検証、結果整形、プランナーのアクセス経路を独立して変更できます。

MongoDBのfindコマンドは初期バッチとカーソル識別子を返します。

Firestoreの[クエリカーソルの説明](https://firebase.google.com/docs/firestore/query-data/query-cursors)は、1つのバッチの最後の文書を次のバッチの開始点に使います。

txBASEは物理ページとソートページでこの境界の考え方を採用します。

サーバー側のカーソル寿命やスナップショット分離は主張しません。

MongoDBの[比較述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)は`$eq`、`$gt`、`$gte`、`$lt`、`$lte`、`$ne`、`$in`、`$nin`などを定義しています。

MongoDBの[BSON 比較順序](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)と[`$gt`の型ブラケット規則](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)は別の境界を定義します。

MongoDBはBSON固有の型規則と配列規則でBSON値を比較します。

txBASEは独自の明示的なスカラー規則でデコード済みJSON値を比較します。

したがって、同じ演算子名でも混在型、欠損フィールド、配列、文書に対する結果が同じとは限りません。

MongoDBの[論理述語リファレンス](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)は`$and`、`$or`、`$nor`、`$not`を定義しています。

MongoDBの[`$expr`述語](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)は、同じ文書内の2つのフィールドを比較する式を含め、述語内の式を許可します。

txBASEは有界な式木を実装します。

`{"$gt":["$LEFT","$RIGHT"]}`のような比較リーフを`$and`、`$or`、`$not`で合成できます。

比較オペランドには、1つのオペランドを持つ数値`$abs`式、または2つのオペランドを持つ数値`$add`、`$subtract`、`$multiply`、`$divide`、`$mod`式も含められます。

整数演算は、結果が収まる場合にJSONの整数結果を維持します。

混在型または小数の演算では、有限なJSON数値だけを生成します。

欠損または非数値のフィールド値は、比較不一致になります。

`$`で始まる文字列オペランドはドット区切りフィールド参照です。

それ以外のスカラーオペランドはリテラルです。

どちらかのフィールド参照が欠損している場合、式は一致しません。

フィールド間比較は定数境界のインデックス検索ではないため、式の経路はテーブルスキャンを使います。

正規表現と文書の式は未サポートです。

txBASEは有界な`$all`、`$elemMatch`、`$size`述語をテーブルスキャンで実装します。

`$all`は配列オペランドを受け付け、空のオペランドはどのレコードにも一致せず、空でないオペランドは配列値のフィールドにすべての候補を要求し、`$elemMatch`はすべての条件を1つの配列要素に束ね、`$size`は0以上の指定した長さだけに一致します。

現在のインデックス契約はマルチキーのキーと配列長の統計を定義しないため、これらの述語はインデックス候補になりません。

MongoDBには正規表現や式評価を含む広い[その他の述語分類](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/misc/)もあります。

これらをファイルネイティブなDBFクエリエンジンに追加するには、先にエンコード、資源上限、エラー契約を定義します。

## 2. プランナーの境界

[MongoDB のクエリ最適化ガイド](https://www.mongodb.com/docs/manual/core/query-optimization/)は、述語の選択性とインデックスキーの順序が調査するデータ量に影響する理由を説明します。

選択性の低い`$ne`や`$nin`は、選択性の高い等値述語と同じようにはインデックスの恩恵を受けないことも説明しています。

txBASEはレコードスキャンをクエリ実行器の参照経路として保ちます。

パス対応のクエリエントリポイントは、同じフィルター、ソート、プロジェクション、スキップ、リミットのパイプラインを適用する前に、外部スカラーキーまたは複合キーの等値、範囲、順序付き走査を試します。

プランナーは`explain_query_at`を通じて`TableScan`、`EqualityIndex`、`CompoundEqualityIndex`、`CompoundEqualityPrefixIndex`、`IndexIntersection`、`RangeIndex`、`OrderedIndex`、`OrderedIndexPrefix`、`CompoundOrderedIndex`を報告します。

単一テーブルHTTPサーバーは`QUERY /explain`で同じ説明を公開します。

応答は`{"plan": {"kind": "table_scan"}}`または選択した名前、フィールド、方向を持つタグ付きインデックス計画です。

有効なインデックスサイドカーがある場合、応答には`cost`オブジェクトも含まれます。

`explain_query_details_at`と`cost`オブジェクトは、選択したアクセス経路の決定的な行相当作業量を公開します。

プランナーは候補数、インデックス走査、残りのソート作業を主順位とし、論理ページ作業を決定的なタイブレークに使って経路を選びます。
インデックス積集合に対する既存の候補選択規則を変えずに、追加項目でレコード読み取り数とフィルター評価数を公開します。

`candidate_rows`は、選択したアクセス経路の後に残る正確な候補数です。

`index_traversal`は、選択したサイドカーの対数探索作業を有界に表します。

`index_page_reads`は、JSONサイドカーを一度読み込むために必要な4 KiB論理ページ数を推定し、テーブルスキャンでは0になります。

`record_reads`はDBFから読み取る候補レコード数を推定します。

等値積集合では、レコード番号の積集合に先立つ候補リストの読み取り数も推定します。

`record_page_reads`は、そのレコードが触れるDBFの論理ページ数を推定します。

`filter_evaluations`は、通常のフィルターパイプラインへ渡す候補レコード数です。

`sort_work`は、順序付きインデックスのプレフィックスを適用した後に残るメモリ内比較作業を推定します。

`total`は、これらすべての項目を飽和加算した値です。

各値は決定的な行相当の推定値であり、経過時間の計測値ではありません。

サイドカーが欠損、古い、壊れている、または意味上サポートされない場合、サイドカー統計を利用できないため、プランナーは`TableScan`へフォールバックし、`cost`を省略します。

この説明は記述的なもので、速度向上を保証しません。

欠損、古い、壊れた、意味上サポートされないサイドカーは、任意の高速化構造であるため`TableScan`にフォールバックします。

インデックスをクエリ実行へ接続するには、パーサー変更だけでは足りません。

キーのエンコード、nullと欠損フィールドの規則、重複順序、更新時の保守、復旧レコード、古いインデックスの検出、プランナー方針が必要です。

現在のプランナーは、トップレベルの直接等値、複合キーの完全一致等値、複合等値プレフィックス候補、片側ごとに1つの境界を持つ範囲述語、単一フィールドの順序付き走査、複合等値プレフィックス範囲述語、正確な等値プレフィックスの後でインデックスサフィックスに一致する単一キーまたは複合ソート要求を扱います。

複数の利用可能な単一フィールド等値インデックスがある場合、プランナーは各単独インデックス候補とレコード番号による積集合候補を比較し、最も低い決定的な行相当コストを選びます。

プランナーはサイドカーのアクティブレコード数と各単一フィールドインデックスの異なるキー数を使い、候補リストを読み込む前に等値のカーディナリティを推定します。

正確な候補リストを推定選択性の順で処理します。

これにより、述語ごとに期待カーディナリティが異なる場合の重複した所属確認を抑えます。

等値推定は一様分布を仮定します。

単一フィールドの範囲インデックスは永続化された等深度ヒストグラムを使い、重なるバケットのレコード数を合計します。

複合インデックスは、先行するすべてのインデックスフィールドに正確な等値述語があり、次のインデックスフィールドに範囲述語がある場合に範囲候補を提供できます。

複合候補経路はインデックスの範囲要素を絞り込み、通常のクエリフィルターパイプラインを実行する前に物理レコード順で返します。

これらの統計は、`QUERY /explain`で公開する決定的な推定値に使います。

プランナーは、テーブルスキャンのアクティブレコード数とインデックス経路の正確な候補数を比較します。
選択したサイドカーのエントリ数から求めた有界な対数走査項を加え、経路が要求された順序の一部しか提供しない場合はメモリ上のソート作業も加えます。

説明には候補レコードの読み取り数とフィルター評価数も含めます。
等値積集合では、レコード番号の積集合に先立つ候補リストの読み取り数を推定します。

アクセス経路の候補構築は`src/query/planner.rs`に置き、コスト計算は`src/query/planner_cost.rs`に分離しています。

これはローカルな計画ロジックであり、MongoDBプランナー互換性ではありません。

複数キーソートでは、単一フィールドインデックスが最初のキーの順序を提供します。

単一キーのソートでも、インデックスの先行フィールドにすべて完全な等値条件があれば、複合インデックスの後続フィールドを利用できます。

実行器は最初のキーが等しいグループ内で残りのキーをメモリ上でソートします。

昇順または混在方向の複合インデックスは、要求されたソートフィールドが正確な等値プレフィックスの後でインデックスフィールドに一致するとき、完全な順序を提供します。

プランナーはインデックス順とその完全な逆順を受け付けます。

そのため対応する混在方向の要求でもメモリ上のソートを避けられます。

等値、範囲、順序付きアクセス経路が同時に有効な場合、プランナーはテーブルスキャンと有効な各インデックス経路の決定的な作業量を比較します。

テーブルスキャンの推定値にはアクティブレコード数を使います。

インデックス経路の推定値には正確な候補数、候補レコードの読み取り数、フィルター評価数を使い、選択したサイドカーごとに有界な対数走査項を1つ加え、経路が要求された順序の一部を実行器に残す場合はソート作業を加えます。

積集合では、選択したサイドカーごとの走査項を合計します。

要求された順序をそのまま提供する順序付き経路にはソート項を加えません。

候補数が同じ場合、複合定義は最短の定義、安定した名前の順で選びます。

コストが一致する場合は、複合等値プレフィックスによる前処理よりもテーブルスキャンと既存のアクセス経路を優先します。

プランナーは、永続化したJSONインデックスとDBFバイト列に対して、4 KiB論理ページの決定的なモデルを使います。
候補レコードのページ数はページマップを具体化しないため、有界な最悪値として推定します。
ファイルシステムの遅延、メモリ、キャッシュ、照合、複合範囲の選択性はこの契約に含めません。

MongoDBの現在の指針は、複数フィールドを繰り返し検索する場合に複合インデックスを推奨しています。

txBASEの積集合はローカルな候補削減機能です。

MongoDBのプランナー互換性や、将来の複合インデックス契約を置き換えるものではありません。

MongoDBの[複合インデックスのソート順に関する説明](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)と[等値、ソート、範囲の指針](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)は、完全な複合インデックスプランナーがフィールド順を定義すべき理由を示します。

txBASEは現在、アクティブレコード数、等値用の一様な異なるキー数推定、単一フィールドの範囲ヒストグラム、複合等値プレフィックス候補と複合等値プレフィックス範囲候補、複合定義ごとの方向メタデータ、スキャン、インデックス走査、論理インデックスページ、候補レコードの読み取り、論理レコードページ、フィルター評価、残りのソート作業に対する説明可能なコスト推定を持ちます。

コストモデルは論理4 KiBページ単位で物理レイアウトを考慮しますが、実際のファイルシステムとキャッシュの動作は契約外です。

等値積集合は有界な候補前処理であり、カバードクエリやエンドツーエンドの高速化を主張するものではありません。

直接の等値結合は、64候補ペアのネステッドループ境界を保ちます。
等値キーの出現数からフィルター前の結合基数を推定し、ロード済みバイト長からDBFの論理ページ数を数え、推定候補行数とプロジェクション幅から出力マテリアライズ作業を加える、別の決定的なコストモデルを使います。

直接および多段結合のモデルは、単一テーブルの`QUERY /explain`コストオブジェクトには含めません。

多段結合ステージは、推定基数、マテリアライズした行幅、論理ページ入力を各中間ステージへ渡し、ハッシュまたはインデックス検索を選択します。

条件を満たす非`full`の多段等値ステージは、マテリアライズした中間行をソートし、ロード済みの新しいテーブルの行を鮮度検証済みで等値条件と一致するordered indexを通じて消費し、その有界なソート作業を推定値へ含めるordered merge経路も比較できます。

結合モデルは論理ページとメモリ内作業に基づくため、ファイルシステムのレイテンシー、キャッシュ状態、ページ再利用は契約外です。

ロードマップでは、クエリ文書が実装戦略を暗黙に指定しないよう、インデックス設計をクエリ構文から分離しています。

## 3. 今後のクエリ作業

次の項目には個別の公開契約が必要です。

1. 欠損、null、照合、複合範囲の選択性規則を明示した完全な式評価と、より精密なコストベースのインデックス選択。
2. 現在の有界な集約契約を超える追加の集約ステージとアキュムレータであり、`$group`、`$bucket`、`$sortByCount`の拡張や、それらの数値式に対する有界なメモリ動作を含む。
3. ファイルシステムとキャッシュを考慮したmerge計画と、より広い結合意味論。
4. `AsyncQueryStream`に対するホスト固有のスケジューリング、バックプレッシャー、タイムアウト、キャンセル、転送の実装。
5. 小さな参照評価器との微分テスト。

これらの契約ができるまでは、レコードスキャンを単純な参照実行モデルとして保ちます。

## 主な参照先

- [MongoDB query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [MongoDB comparison predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/comparison/)
- [MongoDB logical predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/logical/)
- [MongoDB `$expr` predicate](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [MongoDB array predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [MongoDB `$all` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/all/)
- [MongoDB `$elemMatch` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/elemmatch/)
- [MongoDB `$size` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/size/)
- [MongoDB find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)
- [MongoDB query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [MongoDB BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [MongoDB `$gt` type bracketing](https://www.mongodb.com/docs/manual/reference/operator/query/gt/)
- [MongoDB compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [MongoDB equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [SQLite query planning](https://www.sqlite.org/queryplanner.html)
