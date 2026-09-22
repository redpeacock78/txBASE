# SQLite のテストと品質モデル

SQLiteはテストの広さと失敗時の規律に関する有用な参照先です。

txBASEはSQLiteのカバレッジ、テスト量、リリースプロセスを主張しません。

この文書には、現在のプロジェクト規模へ持ち込む価値のある実践を記録します。

## 1. SQLite がテストするもの

SQLite公式の[テスト概要](https://sqlite.org/testing.html)は、独立して開発された複数のテストシステムと、複数の失敗分類を説明しています。

公開されている一覧には次が含まれます。

- パーサーと動作の回帰テスト。
- 境界値テストと定義済み制限のテスト。
- 壊れたデータベースファイルのテスト。
- メモリ不足とI/Oエラーの注入。
- クラッシュと電源断のテスト。
- SQLとデータベースファイルのファジング。
- 最適化を無効にした場合との比較テスト。
- リソースリーク検査、アサーション、Valgrind、未定義動作の検査。

SQLiteはSQL Logic Testによる微分テストも使い、データベースエンジン間で結果を比較します。

正確なハーネスは異なりますが、この分類はどのストレージエンジンにも使えるテストマトリクスを示します。

## 2. SQLite のテストハーネス

SQLiteのページは、4つの主要な系列を説明しています。

| ハーネス | SQLite が説明する目的 |
| --- | --- |
| TCL tests | 大規模なパラメーター化されたスイートによる主開発テスト |
| TH3 | 公開インターフェースを使う移植可能な C テスト。対象コア構成では分岐と MC/DC のカバレッジを持つ |
| SQL Logic Test | エンジン間で SQL 結果を比較する微分テスト |
| Fuzzers | 壊れた、または通常と異なる SQL とデータベース入力から予期しない動作を発見する |

[TH3 の文書](https://sqlite.org/th3.html)は、対象とするSQLiteコア構成でカバレッジ部分集合が100% の分岐カバレッジと100% のMC/DCに達すると説明しています。

これは長期運用された専用の品質目標です。

txBASEの表面的なパーセント目標としてコピーするものではありません。

形式フィクスチャ、クラッシュテスト、壊れた入力テストなしのカバレッジ数では、重要なストレージリスクを測れません。

SQLiteの[品質管理計画](https://sqlite.org/qmplan.html)は、要件、テストカバレッジ、リリースチェックリスト、障害注入の証拠を1つの割合ではなく関連する品質管理として扱います。

その[要件ページ](https://sqlite.org/requirements.html)は、文書化された動作のテスト可能な記述を、安定して追跡可能な要件識別子に変換します。

txBASEでの実用的な適用は、各形式またはプロトコル規則をフィクスチャ、失敗テスト、再現コマンドに結び付ける小さな契約表です。

現在のリポジトリ内の表は[品質契約マトリクス](quality-matrix.md)です。

これは意図的に追跡補助であり、カバレッジスコアではありません。

## 3. 現在のリポジトリがテストするもの

現在のRustテスト配置は所有範囲ごとにすでに分かれています。

| 領域 | 例 |
| --- | --- |
| DBF 形式 | `src/dbf/tests/format.rs`、`format_codepages.rs`、`format_foxpro.rs` |
| memo サイドカー | `src/dbf/tests/memo.rs`、`memo_foxpro.rs`、`src/dbf/malformed_memo_tests.rs` |
| DBF 更新 | `src/dbf/tests/mutation.rs`、`mutation_model_tests.rs`、`writer_tests.rs` |
| 永続化 | `src/dbf/tests/persistence.rs`、`src/dbf/tests/recovery.rs`、`recovery_fault_tests.rs`、`src/dbf/recovery.rs` |
| 保守 | `src/dbf/tests/maintenance.rs`、`src/dbf/schema.rs`、`src/dbf/maintenance.rs` |
| 壊れた入力 | `src/dbf/malformed_tests.rs`、`src/dbf/parser_fuzz_tests.rs`、`src/query/malformed_tests.rs`、`src/transaction/malformed_tests.rs`、`src/xbf/malformed_tests.rs`、`tests/corpus/xbf/` |
| クエリと HTTP | `src/query/tests.rs`、`src/query/array_predicate_tests.rs`、`src/query/cursor_tests.rs`、`src/query/aggregation_tests.rs`、`src/server/tests.rs`、`src/server/catalog_tests.rs`、`src/server/range.rs` |
| トランザクション | `src/transaction/tests.rs`、`src/transaction/malformed_tests.rs`、`src/server/tests.rs`、`src/server/catalog_tests.rs` |

リポジトリは`tests/fixtures/`に外部形式のフィクスチャを、`tests/corpus/`に壊れた入力コーパスを保持します。

## 4. txBASE の品質層

新しいストレージ動作は、契約を証明できる最小の層に追加します。

### 形式とパーサーのテスト

有効なヘッダー、記述子の幅、フィールドフラグ、バイト順、レコード境界、削除マーカー、サイドカーポインター、末尾マーカーをテストします。

切り詰められた長さや一貫しない長さをエラーとしてテストします。

### モデルテスト

小さなメモリ内参照モデルに対する更新列を生成します。

各操作の後にアクティブレコード、削除レコード、自動インクリメント値、memoポインター、拒否された書き込みを比較します。

1つのフィクスチャでは表せない相互作用を検出できます。

### 壊れた入力のテスト

DBF、memo、WAL、JSON入力について決定的なコーパスを保ちます。

検査する性質はpanicしないこと、解析が有界であること、有用なエラーまたは安全な拒否です。

壊れた入力が範囲外読み取り、部分的な成功書き込み、不完全な最終レコードを超える意図しないWAL切り詰めを起こしてはいけません。

### 永続化と復旧のテスト

各境界に障害を置きます。

1. WAL追記の前。
2. WAL追記後、同期の前。
3. 同期後、DBF置換の前。
4. DBF置換後、サイドカー置換の前。
5. すべての置換後、掃除の前。
6. インデックス更新後、WAL掃除の前。

復旧は冪等でなければなりません。

すでに適用した対象を二重に適用してはいけません。

基底が異なる差分は拒否します。

`src/dbf/tests/recovery.rs::recovery_replays_the_dbf_and_index_target_from_one_wal`は、永続的な`TXDI`対象を使い、DBF置換後かつインデックス未置換の境界を検査します。

`src/query/planner_tests.rs::uses_a_valid_equality_index_and_preserves_scan_results`は、単一インデックスの等値、等値積集合、空の積集合、範囲、順序付き走査、テーブルスキャンとの同値を検査します。

`src/query/planner_tests.rs::orders_equality_intersection_by_index_statistics`は、一様な異なるキー数推定を検査し、統計順の結果をテーブルスキャンと比較します。

`src/query/planner_tests.rs::chooses_the_lowest_cost_equality_candidate`は、積集合に走査項が加わる場合に単独等値インデックスを選び、テーブルスキャン結果を保つことを検査します。

`src/index_tests.rs::builds_and_loads_an_external_scalar_index`は、プランナーのコストに使うエントリ由来の走査推定を検査します。

`src/query/planner_tests.rs::chooses_the_lowest_cost_range_candidate`は、ヒストグラム順の候補構築、正確な範囲候補コストの選択、テーブルスキャンとの同値を検査します。

`src/query/planner_compound_tests.rs::uses_a_compound_range_after_an_equality_prefix`は、正確な複合等値プレフィックス後の範囲候補を検査し、選択結果をテーブルスキャンと比較します。

`src/query/planner_compound_tests.rs::uses_a_compound_index_for_exact_equality`は、複合インデックスによる複数フィールドの完全一致の等値検索を検査し、結果をテーブルスキャンと比較します。

`src/query/planner_compound_tests.rs::uses_a_compound_index_for_an_equality_prefix`は、複合インデックスによる等値プレフィックス検索を検査し、結果をテーブルスキャンと比較します。

`src/query/planner_compound_tests.rs::uses_a_compound_index_for_a_multi_field_equality_prefix`は、先頭から続く複数フィールドの等値プレフィックス検索と残余述語を検査し、先頭フィールドを飛ばしたプレフィックスを拒否して、両方の結果をテーブルスキャンと比較します。

`src/query/planner_tests.rs::uses_an_ordered_index_prefix_for_multi_key_sort`は、最初のキーのインデックス走査、二次キーの同値ソート、テーブルスキャンとの同値を検査します。

`src/query/planner_tests.rs::uses_a_compound_index_for_multi_key_sort`は、昇順の複合キーのプレフィックス順、完全な逆順走査、混在方向のフォールバック、テーブルスキャンとの同値を検査します。

`src/query/planner_compound_tests.rs::uses_a_mixed_direction_compound_index`は、フィールドごとの方向、完全な逆順走査、未サポート方向の組み合わせ、テーブルスキャンとの同値を検査します。

`src/query/planner_compound_tests.rs::chooses_a_compound_sort_index_with_the_smallest_equality_prefix`は、等値プレフィックス候補数と、テーブルスキャンと同値の複合計画を検査します。

`src/query/planner_compound_tests.rs::chooses_a_table_scan_for_a_non_selective_index`は、すべてのアクティブレコードを返すインデックスを拒否する有界コストの同点処理を検査します。

`src/query/aggregation_tests/accumulator_tests.rs::groups_filtered_records_with_count_and_integer_sum`は、group内の`$count`と整数`$sum`を検査します。

`src/query/aggregation_tests/pipeline_tests.rs::filters_group_output_before_projection_and_sorting`は、有界なグループ後`$match`を検査します。

`src/query/aggregation_tests/accumulator_tests.rs::sums_fractional_and_integer_numbers`は、整数と小数が混在する`$sum`入力と、整数だけの入力で整数を返す規則を検査します。

`src/query/aggregation_tests/accumulator_tests.rs::groups_numeric_average_and_returns_null_for_missing_values`は`$avg`を検査します。

`src/query/aggregation_tests/accumulator_tests.rs::groups_comparable_extremes_and_returns_null_for_missing_values`は`$min`と`$max`を検査します。

`src/query/aggregation_tests/input_stage_tests.rs::unwinds_array_values_before_grouping`は、配列展開、空配列、欠損フィールド、明示的な`null`を検査します。

`src/query/aggregation_tests/input_stage_tests.rs::preserves_unwind_order_for_array_accumulators`は、`$unwind`後の入力順と配列順を検査します。

`src/query/aggregation_tests/input_stage_tests.rs::applies_input_stages_in_listed_order`は、グループ化前に入力用の`$limit`、`$sort`、`$skip`を記載順で実行することを検査します。

`src/query/aggregation_tests/input_stage_tests.rs::filters_unwound_records_before_grouping`は、`$unwind`後の入力用`$match`で展開済みレコードをフィルターすることを検査します。

`src/query/aggregation_tests/input_stage_tests.rs::rejects_non_array_unwind_values`と`bounds_unwound_records`は、スカラー値の拒否と10,000件の展開上限を検査します。

`src/query/join_strategy.rs::chooses_nested_loop_for_small_join_inputs`、`chooses_hash_for_large_unindexed_inputs`、`chooses_index_nested_loop_when_the_outer_side_is_small`は、有界な等値結合コストの選択を検査します。
`chooses_hash_when_index_fanout_is_expensive`と`chooses_merge_for_large_dual_indexed_inputs`も同じ選択を検査します。

`src/query/join_merge.rs::scans_equal_key_runs_and_maps_them_to_outer_positions`は、順序付きキーを一度走査するmerge経路を検査します。

`src/query/join_index_tests.rs::large_single_key_join_uses_fresh_ordered_indexes`は、直接のordered index経路を検査します。

`src/query/join_index_tests.rs::large_full_join_preserves_unmatched_rows_with_ordered_indexes`は、full結合のmerge経路と、両側で一致しない行を検査します。

同じテストの複合キーケースは、直接の複合ordered index経路を検査します。

`src/query/join_index_tests.rs::chained_single_key_join_uses_a_fresh_foreign_index`と`chained_right_single_key_join_uses_a_fresh_foreign_index`は、多段ステージにおける単一キーのインデックス経路を検査します。
`chained_compound_join_uses_a_fresh_foreign_index`と`chained_right_compound_join_uses_a_fresh_foreign_index`は、複合キーのインデックス経路を検査します。

`src/query/join_tests.rs`は、出力順と連鎖ステージの意味論を検査します。

`src/query/field_expression_tests.rs`は、ドット区切りフィールド参照、欠損または非数値オペランド、有界な数値`$abs`、`$add`/`$subtract`/`$multiply`/`$divide`/`$mod`、0による除算と剰余、壊れたまたは未サポートの`$expr`文書を検査します。

`src/query/array_predicate_tests.rs`は、有界な`$all`、`$elemMatch`、正確な`$size`照合、同一要素への条件束縛、壊れた配列述語文書を検査します。

`src/query/aggregation_tests/accumulator_tests.rs::evaluates_bounded_numeric_accumulator_expressions`は、欠損値と非数値を含む、`$sum`と`$avg`で共有する有界な数値式を検査します。

### 互換性テスト

可能な場合は独立した読み書き実装からフィクスチャを取得します。

現在のリポジトリは、実装したフィールドとmemo経路についてdBASE III、dBASE IV、Visual FoxProのカバレッジを含みます。

新しい型またはコードページごとに、バイトレベルのフィクスチャとJSON往復検査が必要です。

### HTTP テスト

メソッド、パス、コンテンツタイプ、応答ステータス、ヘッダー、本文、壊れたJSON、未サポート演算子、範囲リクエスト、古いwriterの失敗をテストします。

HTTPテストは、直接のWALとDBF復旧テストの代わりにはなりません。

## 5. CI 契約

`.github/workflows/ci.yml`のワークフローはUbuntu、macOS、Windowsで実行します。

各マトリクスジョブは次を実行します。

```bash
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

ドキュメント検査は、ソースをリファクタリングした後もMarkdownに書かれた具体的な`src/`および`tests/`のパスが存在することを検証します。

ワイルドカードを含む例は説明用として扱い、この検査では展開しません。

これが現在のgreen gateです。

SQLiteの完全なリリースプロセスより意図的に小さくしています。

長時間のファジング、soak、微分、障害注入ジョブは、入力、タイムアウト、成果物、失敗再現の契約を定義してから追加します。

## 6. 提案する次の検査

次の順序にするとフィードバックループを短く保てます。

1. 新しく受け付けるDBF記述子またはサイドカー形式ごとにフィクスチャを追加する。
2. 複数段階の更新列に参照モデルのケースを追加する。
3. 壊れた入力コーパスとpanicなし検査を拡張する。
4. すべてのWALレコード種別と置換境界について決定的な復旧テストを拡張する。
5. パーサーとクエリ検証に有界ファジング対象を追加する。
6. 参照クエリ評価器ができてから微分検査を追加する。

カバレッジ率は、これらの契約の代わりにはなりません。

新しい形式、クエリ、永続化、HTTP境界を追加する変更では、同じ変更でマトリクスも更新します。

## 主な参照先

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite quality management](https://sqlite.org/qmplan.html)
- [SQLite requirements](https://sqlite.org/requirements.html)
- [SQLite limits](https://sqlite.org/limits.html)
