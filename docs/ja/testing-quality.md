# SQLite のテストと品質モデル

SQLite はテストの広さと失敗時の規律に関する有用な参照先です。

txBASE は SQLite のカバレッジ、テスト量、リリースプロセスを主張しません。

この文書には、現在のプロジェクト規模へ持ち込む価値のある実践を記録します。

## 1. SQLite がテストするもの

SQLite 公式の[テスト概要](https://sqlite.org/testing.html)は、独立して開発された複数のテストシステムと、複数の失敗分類を説明しています。

公開されている一覧には次が含まれます。

- パーサーと動作の回帰テスト。
- 境界値テストと定義済み制限のテスト。
- 壊れたデータベースファイルのテスト。
- メモリ不足と I/O エラーの注入。
- クラッシュと電源断のテスト。
- SQL とデータベースファイルのファジング。
- 最適化を無効にした場合との比較テスト。
- リソースリーク検査、アサーション、Valgrind、未定義動作の検査。

SQLite は SQL Logic Test による微分テストも使い、データベースエンジン間で結果を比較します。

正確なハーネスは異なりますが、この分類はどのストレージエンジンにも使えるテストマトリクスを示します。

## 2. SQLite のテストハーネス

SQLite のページは、四つの主要な系列を説明しています。

| ハーネス | SQLite が説明する目的 |
| --- | --- |
| TCL tests | 大規模なパラメーター化されたスイートによる主開発テスト |
| TH3 | 公開インターフェースを使う移植可能な C テスト。対象コア構成では分岐と MC/DC のカバレッジを持つ |
| SQL Logic Test | エンジン間で SQL 結果を比較する微分テスト |
| Fuzzers | 壊れた、または通常と異なる SQL とデータベース入力から予期しない動作を発見する |

[TH3 の文書](https://sqlite.org/th3.html)は、対象とする SQLite コア構成でカバレッジ部分集合が 100% の分岐カバレッジと 100% の MC/DC に達すると説明しています。

これは長期運用された専用の品質目標です。

txBASE の表面的なパーセント目標としてコピーするものではありません。

形式フィクスチャ、クラッシュテスト、壊れた入力テストなしのカバレッジ数では、重要なストレージリスクを測れません。

SQLite の[品質管理計画](https://sqlite.org/qmplan.html)は、要件、テストカバレッジ、リリースチェックリスト、障害注入の証拠を一つの割合ではなく関連する品質管理として扱います。

その[要件ページ](https://sqlite.org/requirements.html)は、文書化された動作のテスト可能な記述を、安定して追跡可能な要件識別子に変換します。

txBASE での実用的な適用は、各形式またはプロトコル規則をフィクスチャ、失敗テスト、再現コマンドに結び付ける小さな契約表です。

現在のリポジトリ内の表は[品質契約マトリクス](quality-matrix.md)です。

これは意図的に追跡補助であり、カバレッジスコアではありません。

## 3. 現在のリポジトリがテストするもの

現在の Rust テスト配置は所有範囲ごとにすでに分かれています。

| 領域 | 例 |
| --- | --- |
| DBF 形式 | `src/dbf/tests/format.rs`、`format_codepages.rs`、`format_foxpro.rs` |
| memo サイドカー | `src/dbf/tests/memo.rs`、`memo_foxpro.rs`、`src/dbf/malformed_memo_tests.rs` |
| DBF 更新 | `src/dbf/tests/mutation.rs`、`mutation_model_tests.rs`、`writer_tests.rs` |
| 永続化 | `src/dbf/tests/persistence.rs`、`recovery_fault_tests.rs`、`src/dbf/recovery.rs` |
| 保守 | `src/dbf/tests/maintenance.rs`、`src/dbf/schema.rs`、`src/dbf/maintenance.rs` |
| 壊れた入力 | `src/dbf/malformed_tests.rs`、`src/dbf/parser_fuzz_tests.rs`、`src/query/malformed_tests.rs`、`src/transaction/malformed_tests.rs` |
| クエリと HTTP | `src/query/tests.rs`、`src/query/cursor_tests.rs`、`src/query/aggregation_tests.rs`、`src/server/tests.rs`、`src/server/range.rs` |
| トランザクション | `src/transaction/tests.rs`、`src/transaction/malformed_tests.rs`、`src/server/tests.rs` |

リポジトリは `tests/fixtures/` に外部形式のフィクスチャを、`tests/corpus/` に壊れた入力コーパスを保持します。

## 4. txBASE の品質層

新しいストレージ動作は、契約を証明できる最小の層に追加します。

### 形式とパーサーのテスト

有効なヘッダー、記述子の幅、フィールドフラグ、バイト順、レコード境界、削除マーカー、サイドカーポインター、末尾マーカーをテストします。

切り詰められた長さや一貫しない長さをエラーとしてテストします。

### モデルテスト

小さなメモリ内参照モデルに対する更新列を生成します。

各操作の後にアクティブレコード、削除レコード、自動インクリメント値、memo ポインター、拒否された書き込みを比較します。

一つのフィクスチャでは表せない相互作用を検出できます。

### 壊れた入力のテスト

DBF、memo、WAL、JSON 入力について決定的なコーパスを保ちます。

検査する性質は panic しないこと、解析が有界であること、有用なエラーまたは安全な拒否です。

壊れた入力が範囲外読み取り、部分的な成功書き込み、不完全な最終レコードを超える意図しない WAL 切り詰めを起こしてはいけません。

### 永続化と復旧のテスト

各境界に障害を置きます。

1. WAL 追記の前。
2. WAL 追記後、同期の前。
3. 同期後、DBF 置換の前。
4. DBF 置換後、サイドカー置換の前。
5. すべての置換後、掃除の前。
6. インデックス更新後、WAL 掃除の前。

復旧は冪等でなければなりません。

すでに適用した対象を二重に適用してはいけません。

基底が異なる差分は拒否します。

`src/dbf/tests/persistence.rs::recovery_replays_the_dbf_and_index_target_from_one_wal` は、永続的な `TXDI` 対象を使い、DBF 置換後かつインデックス未置換の境界を検査します。

`src/query/planner_tests.rs::uses_a_valid_equality_index_and_preserves_scan_results` は、単一インデックスの等値、等値積集合、空の積集合、範囲、順序付き走査、テーブルスキャンとの同値を検査します。

`src/query/planner_tests.rs::orders_equality_intersection_by_index_statistics` は、一様な異なるキー数推定を検査し、統計順の結果をテーブルスキャンと比較します。

`src/query/planner_tests.rs::orders_multiple_range_access_by_histogram_estimate` は、ヒストグラム順の範囲アクセスを検査し、選択結果をテーブルスキャンと比較します。

`src/query/planner_tests.rs::uses_an_ordered_index_prefix_for_multi_key_sort` は、最初のキーのインデックス走査、二次キーの同値ソート、テーブルスキャンとの同値を検査します。

`src/query/planner_tests.rs::uses_a_compound_index_for_multi_key_sort` は、昇順の複合キーのプレフィックス順、完全な逆順走査、混在方向のフォールバック、テーブルスキャンとの同値を検査します。

`src/query/planner_compound_tests.rs::uses_a_mixed_direction_compound_index` は、フィールドごとの方向、完全な逆順走査、未サポート方向の組み合わせ、テーブルスキャンとの同値を検査します。

`src/query/planner_compound_tests.rs::chooses_a_compound_sort_index_with_the_smallest_equality_prefix` は、等値プレフィックス候補数と、テーブルスキャンと同値の複合計画を検査します。

`src/query/planner_compound_tests.rs::chooses_a_table_scan_for_a_non_selective_index` は、すべてのアクティブレコードを返すインデックスを拒否する有界コストの同点処理を検査します。

`src/query/join_strategy.rs::chooses_nested_loop_for_small_join_inputs`、`chooses_hash_for_large_join_inputs`、`chooses_index_nested_loop_for_large_indexed_inputs` は、等値結合の決定的な戦略しきい値を検査します。
`large_single_key_join_uses_a_fresh_foreign_index` は、直接のインデックス経路を検査します。
`chained_single_key_join_uses_a_fresh_foreign_index`、`chained_right_single_key_join_uses_a_fresh_foreign_index`、`chained_compound_join_uses_a_fresh_foreign_index` は、多段ステージにおけるインデックス経路を検査します。
結合テストは、出力順と連鎖ステージの意味論を検査します。

`src/query/field_expression_tests.rs` は、ドット区切りフィールド参照、欠損オペランド、壊れたまたは未サポートの `$expr` 文書を検査します。

### 互換性テスト

可能な場合は独立した読み書き実装からフィクスチャを取得します。

現在のリポジトリは、実装したフィールドと memo 経路について dBASE III、dBASE IV、Visual FoxPro のカバレッジを含みます。

新しい型またはコードページごとに、バイトレベルのフィクスチャと JSON 往復検査が必要です。

### HTTP テスト

メソッド、パス、コンテンツタイプ、応答ステータス、ヘッダー、本文、壊れた JSON、未サポート演算子、範囲リクエスト、古い writer の失敗をテストします。

HTTP テストは、直接の WAL と DBF 復旧テストの代わりにはなりません。

## 5. CI 契約

`.github/workflows/ci.yml` のワークフローは Ubuntu、macOS、Windows で実行します。

各マトリクスジョブは次を実行します。

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

これが現在の green gate です。

SQLite の完全なリリースプロセスより意図的に小さくしています。

長時間のファジング、soak、微分、障害注入ジョブは、入力、タイムアウト、成果物、失敗再現の契約を定義してから追加します。

## 6. 提案する次の検査

次の順序にするとフィードバックループを短く保てます。

1. 新しく受け付ける DBF 記述子またはサイドカー形式ごとにフィクスチャを追加する。
2. 複数段階の更新列に参照モデルのケースを追加する。
3. 壊れた入力コーパスと panic なし検査を拡張する。
4. すべての WAL レコード種別と置換境界について決定的な復旧テストを拡張する。
5. パーサーとクエリ検証に有界ファジング対象を追加する。
6. 参照クエリ評価器ができてから微分検査を追加する。

カバレッジ率は、これらの契約の代わりにはなりません。

新しい形式、クエリ、永続化、HTTP 境界を追加する変更では、同じ変更でマトリクスも更新します。

## 主な参照先

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite quality management](https://sqlite.org/qmplan.html)
- [SQLite requirements](https://sqlite.org/requirements.html)
- [SQLite limits](https://sqlite.org/limits.html)
