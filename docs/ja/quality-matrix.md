# 品質契約マトリクス

このマトリクスは、SQLite から得た要件管理の知見を、小さなリポジトリ内の追跡表に変換します。

動作の主張を実装の証拠と決定的な検査へ結び付けます。

カバレッジ率、SQLite と同等の品質、すべてのテストの一覧を主張するものではありません。

## マトリクスの読み方

各識別子は txBASE 内でだけ使います。

`Current` は契約が実装され、引用した検査で表現されていることを意味します。

`Boundary` は動作を意図的に制限し、その制限も契約の一部であることを意味します。

`Future` は話題を文書化しているだけで、実装済み機能として説明してはいけないことを意味します。

権威ある CI コマンドは次のとおりです。

```bash
bash scripts/check-doc-translations.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

ワークフローは Ubuntu、macOS、Windows でこのゲートを実行します。

## 現在の契約

| ID | 契約 | 実装またはフィクスチャの証拠 | 決定的な検査 | 状態 |
| --- | --- | --- | --- | --- |
| DBF-001 | 宣言済みヘッダー、記述子、レコード長、削除マーカーを境界検査する。 | `src/dbf/parser.rs`; `tests/corpus/dbf/` | `src/dbf/malformed_tests.rs::rejects_malformed_dbf_corpus` と形式テスト | Current |
| DBF-002 | 上流の Windows-1251 テーブルを含む、サポートする dBASE III、dBASE IV、Visual FoxPro のフィクスチャが宣言されたフィールド境界を往復し、スキーマメタデータに宣言済みコードページ名を公開する。 | `tests/fixtures/external-*.dbf.hex`; `src/dbf/compatibility_tests.rs`; `src/dbf/codec.rs`; `src/dbf/encoding_name_tests.rs` | `reads_and_writes_a_pinned_external_*_fixture`; `reads_and_writes_a_pinned_external_cp1251_fixture`; `reports_names_for_supported_language_drivers` | Current |
| DBF-003 | 四つの Visual FoxPro CJK driver ID、legacy dBASE の `0x13`、`0x4d`、`0x4e`、`0x4f` alias、サポートする八つの明示コーデック名が、バイト幅検査、置換読み取り、書き込み拒否、宣言値と実効値の可視化、実効コーデックによるフィールド名のデコードを使う。 | `src/dbf/codec_cjk.rs`; `src/dbf/codec_fields.rs`; `src/dbf/schema.rs`; `src/dbf/cjk_tests.rs`; `src/dbf/cjk_fields_tests.rs`; `src/dbf/upstream_cjk_tests.rs`; `src/dbf/schema_encoding_tests.rs`; `tests/fixtures/cjk-*.dbf.hex`; `tests/fixtures/cjk-field-name-gbk.dbf.hex`; `tests/fixtures/external-javadbf-gbk.dbf.hex`; `tests/fixtures/external-dbase-rs-cp936.dbf.hex`; `tests/fixtures/cjk-explicit-codecs.json` | `reads_and_writes_pinned_cjk_driver_fixtures`; `decodes_and_encodes_declared_cjk_drivers`; `decodes_and_encodes_legacy_cjk_driver_aliases`; `decodes_cjk_field_names_with_the_effective_encoding`; `reads_and_writes_a_pinned_upstream_gbk_fixture`; `reads_and_writes_a_pinned_upstream_cp936_fixture`; `reads_pinned_explicit_cjk_codec_bytes_from_a_dbf_record`; `explicit_euc_jp_and_gb18030_overrides_round_trip`; `explicit_iso_2022_jp_override_round_trips_jis_text`; `strict_shift_jis_override_round_trips_jis_text`; `strict_shift_jis_rejects_cp932_extensions`; `accepts_strict_shift_jis_as_an_explicit_override`; `accepts_iso_2022_jp_as_an_explicit_override` | Current |
| MEM-001 | DBT と FPT のポインター、ブロックサイズ、終端、壊れたサイドカーを選択された memo 形式の範囲内で扱う。 | `src/dbf/memo_file.rs`; `src/dbf/memo_value.rs`; `tests/corpus/memo/` | `src/dbf/tests/memo.rs`、`memo_foxpro.rs`、`malformed_memo_tests.rs` | Current |
| MUT-001 | レコード更新がメモリ内モデルを保ち、未知または曖昧な書き込みを拒否し、不透明フィールドを保持する。 | `src/dbf/mutation.rs`; `src/dbf/mutation_model_tests.rs` | `generated_mutation_sequence_matches_reference_model` と更新テスト | Current |
| WAL-001 | 完全な WAL レコードを検証し、最後の切断されたレコードを panic なしで切り詰めるか破棄する。 | `src/transaction/wal.rs`; `src/dbf/wal.rs`; `tests/corpus/wal/` | `transaction::malformed_tests::rejects_malformed_wal_corpus`; `file_wal_reopens_and_truncates_a_torn_tail` | Current |
| WAL-002 | スナップショット、差分、更新意図、memo、インデックスの復旧を置換境界で冪等にし、壊れたインデックススナップショットを復旧可能な WAL エラーとして扱う。 | `src/dbf/recovery.rs`; `src/dbf/persistence.rs` | `src/dbf/tests/recovery.rs` の復旧ケース; `recovery_fault_tests.rs::keeps_a_wal_with_a_malformed_index_snapshot` | Current |
| WAL-003 | 独立してロードした古い writer が、新しい DBF または memo バイト列を黙って上書きせず拒否される。 | `src/dbf/persistence.rs`; `src/dbf/writer_tests.rs` | `rejects_a_stale_second_writer_without_overwriting_the_first_save` | Current |
| WAL-004 | 読み取り専用 WAL 検査が、WAL を作成、切り詰め、変更せずに完全なレコード境界と切断された末尾を表示し、完全な壊れたレコードは引き続きエラーにする。 | `src/transaction/wal.rs`; `src/main.rs`; `tests/cli.rs`; `docs/ja/dbf-compatibility.md` | `file_wal_inspect_reports_but_does_not_truncate_a_torn_tail`; `file_wal_inspect_does_not_create_a_missing_file`; `wal_inspect_cli_reports_a_torn_tail_without_mutating_the_file` | Current |
| TXN-001 | 低レベルのスナップショットトランザクション管理器が、保持されたメモリまたはファイル WAL のコミット、ロールバック記録から次のトランザクション ID を継続する。ただし DBF やカタログの HTTP ID、MVCC 可視性は提供しない。 | `src/transaction/mod.rs`; `src/transaction/wal.rs` | `snapshot_engine_resumes_transaction_ids_from_wal`; `file_snapshot_engine_resumes_transaction_ids_after_reopen` | Boundary |
| TXN-002 | 単一テーブル DBF WAL コミットが正の永続 ID を割り当て、`TXTI` から復旧し、古いトランザクション状態の writer を拒否し、状態サイドカーをコピーし、HTTP 更新へ ID を公開する。 | `src/dbf/persistence.rs`; `src/dbf/recovery.rs`; `src/dbf/maintenance.rs`; `src/server/etag.rs` | `wal_commit_ids_persist_and_resume_after_reload`; `snapshot_recovery_persists_the_wal_transaction_id`; `copy_table_files_preserves_transaction_state`; `mutation_etag_prevents_lost_update` | Current |
| XBF-001 | 草案 XBF コーデックが割り当てを制限し、ヘッダーとセクションの CRC-32C を検証し、予約外の v1 スカラー型をデコードし、物理削除フラグを保ち、ローカル一意性を強制する。 | `src/xbf/`; `src/xbf/malformed_tests.rs`; `tests/corpus/xbf/`; `docs/ja/xbf.md` | `src/xbf/tests.rs` のフィクスチャ、破損、サイズ制限、制約ケース; `rejects_malformed_xbf_header_corpus`; `rejects_malformed_xbf_sections_and_directory`; `rejects_malformed_xbf_utf8_and_payload_values` | Boundary |
| XBF-002 | XBF スナップショット経路が書き込み前にエンコードし、スナップショットバイトを同期し、対象を置換し、戻る前に親ディレクトリを同期し、通常読み取りで先に保留 WAL を復旧する。 | `src/xbf/persistence.rs` | `writes_and_reads_a_durable_snapshot_path`; `reading_a_snapshot_recovers_a_pending_wal` | Boundary |
| XBF-003 | 完全スナップショット XBF WAL が基準世代と対象世代を記録し、トランザクション層のレコード上限を守り、保留スナップショットを呼び出し側の制限で冪等に適用し、異なる現在世代を拒否する。 | `src/xbf/wal.rs` | `recovers_a_generation_checked_full_snapshot_wal`; `rejects_a_generation_mismatched_xbf_wal`; `rejects_xbf_wal_records_over_the_file_wal_limit` | Boundary |
| XBF-004 | ロード済み DBF レコードが、物理順、削除フラグ、DBF が区別できる null を落とさず有界 XBF 値へ変換され、未サポート値は明示的に失敗する。 | `src/xbf/conversion.rs`; `src/xbf/export_tests.rs` | `converts_a_dbf_fixture_to_xbf_without_dropping_records` | Boundary |
| XBF-005 | 表現可能な XBF テーブルが削除状態を保ったメモリ内 DBF テーブルへ出力され、直接出力は未サポート型と制約を拒否し、スキーマ対応出力は表現可能な制約メタデータを返して保存できる。 | `src/xbf/export.rs`; `src/xbf/export_tests.rs` | `exports_a_representable_xbf_table_to_dbf`; `exports_xbf_constraints_as_schema_metadata`; `saves_xbf_dbf_and_schema_sidecar`; `rejects_nonrepresentable_xbf_dbf_export_types` | Boundary |
| XBF-006 | XBF から DBF への表現可能性をファイル書き込みなしで検査し、発見した全フィールド、レコード問題とスキーマサイドカーの必要性を含める。 | `src/xbf/export.rs`; `src/xbf/export_tests.rs`; `docs/ja/xbf.md` | `reports_all_nonrepresentable_xbf_fields_and_values`; `report_marks_constraint_sidecar_requirement` | Boundary |
| XBF-007 | スキーマを保つ XBF から DBF へのファイル出力が durable `TXSE` ジャーナル、正確な対象基底バイト、ファイルごとの同期と置換、memo サイドカー掃除、永続トランザクション状態置換、既存インデックス更新、部分置換後の DBF 読み取り復旧、外部変更対象の競合拒否、不正ジャーナル拒否を使う。 | `src/dbf/schema_export.rs`; `src/dbf/persistence.rs`; `src/dbf/parser.rs`; `src/index/commit.rs`; `src/dbf/schema_export_tests.rs` | `schema_export_commits_dbf_and_schema_together`; `schema_export_removes_stale_memo_sidecars`; `schema_export_read_recovers_after_dbf_replacement`; `schema_export_read_rejects_an_external_target_change`; `schema_export_rejects_malformed_journal_records` | Boundary |
| IDX-001 | 外部スカラーおよび複合インデックスが再構築可能で、鮮度を検査し、安全でない更新前に拒否され、サポートする更新または復旧後に更新される。 | `src/index.rs`; `src/index/`; `src/index_tests.rs`; `src/dbf/tests/persistence.rs`; `src/dbf/tests/recovery.rs` | `mutation_refreshes_the_sidecar_after_save`; `rejects_an_invalid_index_before_saving_the_dbf`; `replays_operation_intent_when_state_payload_is_missing`; `direct_dbf_change_leaves_the_sidecar_stale_until_rebuild` | Current |
| IDX-002 | インデックス候補が、等値、範囲、複合等値プレフィックス範囲、順序プレフィックス、複合、等値積集合の計画でテーブルスキャン結果を保つ。 | `src/query/planner.rs`; `src/query/planner_tests.rs`; `src/query/planner_compound_tests.rs` | `uses_a_compound_range_after_an_equality_prefix`; `uses_*` と `orders_*` という名前のプランナーテスト | Current |
| IDX-003 | 選択されたテーブルスキャンまたはインデックス計画が、クエリ実行を変えずタグ付き JSON 説明として得られる。 | `src/query/planner.rs`; `src/server/explain.rs`; `docs/ja/query-planning.md` | `explain_endpoint_reports_scan_and_index_plans` | Boundary |
| IDX-004 | 等値、範囲、順序付きアクセス経路が同時に有効な場合、プランナーがレコード数、インデックス走査、ソート作業による有界コストを比較し、非選択的な完全同点ではテーブルスキャンへ戻り、テーブルスキャン結果を保つ。 | `src/query/planner.rs`; `src/query/planner_compound_tests.rs`; `src/query/planner_tests.rs`; `src/index_tests.rs`; `docs/indexes.md` | `chooses_the_access_path_with_fewer_exact_candidates`; `chooses_the_lowest_cost_range_candidate`; `chooses_a_table_scan_for_a_non_selective_index`; `builds_and_loads_an_external_scalar_index` | Boundary |
| QRY-001 | クエリ解析が、MongoDB 互換性を主張せず、文書化した filter、path、projection、sort、update の境界を検証する。 | `src/query/validation.rs`; `src/query/tests.rs` | `parses_query_shape`; `rejects_unknown_query_fields`; `rejects_unknown_and_mixed_projection_operators` | Current |
| QRY-002 | 比較リーフ、一つの数値オペランドを持つ `$abs`、二つの数値オペランドを持つ `$add` / `$subtract` / `$multiply` / `$divide` / `$mod` 式上の有界 `$expr` 論理木、および壊れた JSON クエリコーパスが安全に失敗し、インデックス検索にならない。 | `src/query/field_expression_tests.rs`; `src/query/validation.rs`; `src/query/malformed_tests.rs` | `compares_two_fields_with_expr`; `composes_expression_comparisons_with_boolean_operators`; `compares_numeric_expression_results`; `compares_absolute_expression_results`; `compares_multiplication_expression_results`; `compares_division_expression_results`; `compares_modulo_expression_results`; `rejects_division_by_zero`; `rejects_modulo_by_zero`; `missing_or_non_numeric_expression_fields_do_not_match`; `rejects_numeric_expression_results_that_do_not_fit_json`; `rejects_malformed_json_query_corpus` | Boundary |
| QRY-003 | 物理カーソルが要求ページと先読みレコードで止まり、発行する物理およびソートカーソルがテーブル表現に結び付き、ソートカーソルが一致するキーセット定義を強制し、有界な集約とローカル結合が連鎖ステージ全体で明示的な上限を強制する。 | `src/query/pagination.rs`; `src/dbf/mod.rs`; `src/query/aggregation.rs`; `src/query/aggregation_plan.rs`; `src/query/join/mod.rs`; `src/query/join/execute.rs`; `src/query/join_pipeline.rs`; `docs/ja/aggregation.md`; `docs/ja/joins.md` | `paginates_in_physical_record_order`; `rejects_a_physical_cursor_for_a_changed_snapshot`; `rejects_a_sorted_cursor_for_a_changed_snapshot`; `returns_distinct_values_after_matching`; `filters_group_output_before_projection_and_sorting`; `groups_filtered_records_with_count_and_integer_sum`; `sums_fractional_and_integer_numbers`; `groups_numeric_average_and_returns_null_for_missing_values`; `groups_comparable_extremes_and_returns_null_for_missing_values`; `groups_first_and_last_values_in_physical_order`; `chained_joins_reference_fields_from_prior_stages`; `right_join_keeps_unmatched_right_record` などの結合テスト | Boundary |
| QRY-004 | 借用型および所有型スナップショットクエリストリームが filter、projection、skip、limit を段階的に適用し、ブロッキングまたは再開可能な制御を拒否する。 | `src/query/stream.rs`; `src/query/stream_tests.rs` | `streams_filtered_projected_records_with_bounded_controls`; `snapshot_stream_is_independent_of_later_table_mutations`; `streaming_rejects_blocking_and_resume_controls` | Boundary |
| QRY-005 | 文書化した Unicode 小文字化照合が文字列順を変え、ソートカーソルを同じ照合へ結び付け、互換しないインデックス順を避ける。 | `src/query/ordering.rs`; `src/query/pagination.rs`; `src/query/planner.rs`; `src/query/tests.rs`; `src/query/cursor_tests.rs` | `supports_unicode_lowercase_collation_for_sort_keys`; `sorted_cursor_keeps_collation_in_its_boundary`; プランナーの照合フォールバック | Boundary |
| QRY-006 | 有界スナップショットストリームが複製したテーブルを正の容量の標準ライブラリチャネルの背後で実行し、チャネル満杯時に生成側を停止し、項目順を保ち、消費側キャンセル後に停止する。ランタイム固有非同期トレイトは境界外とする。 | `src/query/stream.rs`; `src/query/stream_tests.rs`; `docs/ja/query-model.md` | `bounded_snapshot_stream_keeps_a_fixed_snapshot`; `bounded_stream_requires_positive_capacity` | Boundary |
| QRY-007 | 等値結合が候補ペア 64 以下では `NestedLoop` を選び、それを超えると有界なハッシュ、インデックス検索、互換性のある ordered merge のコストを比較し、必要な `right` 結合では外側行数を反転して、文書化した外側行の順序と結合意味論を保つ。 | `src/query/join_strategy.rs`; `src/query/join_merge.rs`; `src/query/join_index.rs`; `src/query/join/mod.rs`; `src/query/join/execute.rs`; `src/query/join_pipeline.rs`; `src/index/lookup/mod.rs`; `src/index/lookup/ordered.rs`; `docs/ja/joins.md` | `chooses_nested_loop_for_small_join_inputs`; `chooses_hash_for_large_unindexed_inputs`; `chooses_index_nested_loop_when_the_outer_side_is_small`; `chooses_merge_for_large_dual_indexed_inputs`; `scans_equal_key_runs_and_maps_them_to_outer_positions`; 大きな結合と多段インデックス結合; 直接の複合結合; 結合出力テスト | Boundary |
| CAT-001 | カタログが直下の DBF ファイルだけを検出し、テーブルごとのロードまたは検証失敗を報告する。 | `src/catalog.rs`; `docs/catalog.md` | `cargo test --all-targets --all-features` のカタログおよび結合ケース | Current |
| CAT-002 | カタログサーバーが検出済みスキーマと有界読み取り専用結合を公開し、テーブル間更新を明示的なトランザクション経路に保つ。 | `src/server/catalog.rs`; `docs/catalog.md`; `docs/http-semantics.md` | `catalog_server_query_join_executes_and_exposes_schema` | Boundary |
| CAT-003 | カタログサーバーが名前付きテーブルの GET、HEAD、QUERY、POST、PUT、PATCH、DELETE ルートを公開し、単一テーブルのレコード、ETag、クエリ、更新意味論を再利用する。 | `src/server/catalog.rs`; `src/server/records.rs`; `docs/catalog.md`; `docs/http-semantics.md` | `catalog_server_reads_named_tables_through_record_routes`; `catalog_server_mutates_named_tables_with_single_table_semantics` | Boundary |
| CAT-004 | カタログサーバーが単一テーブルサーバーと同じプランナー説明契約で、名前付きテーブルの選択済みクエリ計画を公開する。 | `src/server/catalog.rs`; `src/server/explain.rs`; `docs/catalog.md`; `docs/http-semantics.md` | `catalog_server_reads_named_tables_through_record_routes` | Boundary |
| CAT-005 | 名前付きテーブル更新操作が一つのカタログジャーナルで複数 DBF にコミットでき、検証失敗をロールバックし、準備済みまたはコミット済みのジャーナル状態を復旧する。 | `src/catalog/transaction.rs`; `src/catalog/journal_tests.rs`; `src/server/catalog_transaction.rs`; `docs/catalog.md`; `docs/http-semantics.md` | `commits_named_operations_across_tables`; `rejects_a_failed_named_transaction_without_persisting_earlier_tables`; カタログジャーナル復旧テスト; `catalog_server_transaction_commits_multiple_named_tables`; `catalog_server_transaction_rolls_back_when_a_named_operation_fails` | Boundary |
| CAT-006 | カタログスコープの `references` メタデータが名前付き更新と複数テーブルトランザクション後の非 null 子値を検証し、孤立した子を残す親更新または論理削除を防ぐ。 | `src/catalog.rs`; `src/catalog/transaction.rs`; `src/server/catalog.rs`; `src/server/records.rs`; `src/dbf/schema_metadata.rs` | `catalog_foreign_keys_validate_mutations_and_parent_removal`; `catalog_server_rejects_orphan_foreign_key_mutations` | Boundary |
| CAT-007 | 複数テーブルのカタログジャーナルコミットが正で単調増加する ID を永続化し、準備済みまたはコミット済みのジャーナル状態から復旧し、カタログスキーマ JSON に公開し、`POST /transaction` の JSON と `X-Txbase-Transaction-Id` で返す。 | `src/catalog/journal.rs`; `src/catalog/journal_tests.rs`; `src/catalog/transaction.rs`; `src/catalog.rs`; `src/catalog/tests.rs`; `src/server/catalog_transaction.rs`; `docs/catalog.md`; `docs/http-semantics.md` | `commits_named_operations_across_tables`; `representation_tag_is_stable_and_changes_with_catalog_schema`; カタログジャーナル復旧テスト; カタログトランザクション HTTP テスト | Current |
| SCH-001 | 任意のスキーマメタデータがアクティブレコードと更新候補を検証し、省略された insert フィールドへスカラー既定値を適用し、従来の DBF バイト列を変えずに有界な複合 primary、unique、テーブルレベル checks をサポートする。 | `src/dbf/schema_metadata.rs`; `src/dbf/mutation.rs`; `src/dbf/schema_metadata_tests.rs` | `loads_schema_metadata_and_enforces_local_constraints`; `schema_defaults_fill_missing_insert_fields`; `composite_unique_constraints_reject_duplicate_keys`; `composite_primary_constraints_require_non_null_unique_keys`; `schema_checks_reject_invalid_candidates`; 古いサイドカーとコピーのテスト | Boundary |
| HTTP-001 | HTTP メソッド、JSON メディアタイプ、QUERY ステータス、応答ヘッダー、更新演算子の境界を明示する。 | `src/server.rs`; `src/server/records.rs`; `src/server/transaction.rs`; `docs/http-semantics.md` | `src/server/tests.rs::query_endpoint_enforces_json_boundary_and_executes` と更新テスト | Current |
| HTTP-002 | サポート対象の単一バイト範囲と永続化失敗が有界な応答を返し、誤ったテーブル状態を置き換えない。 | `src/server/range.rs`; `src/server/tests.rs` | `query_endpoint_handles_single_byte_ranges`; `reloads_disk_state_after_persistence_failure` | Boundary |
| HTTP-003 | 成功したテーブル読み取りが強い ETag を公開し、状態変更ルートが定義された比較、存在するリソースのワイルドカード、412 失敗、部分変更なし、コミット後の新しい ETag を伴う任意の If-Match と If-None-Match を尊重する。 | `src/server.rs`; `src/server/records.rs`; `src/server/transaction.rs`; `src/server/etag.rs`; `src/server/etag_tests.rs`; `docs/http-semantics.md` | `mutation_etag_prevents_lost_update`; `post_put_and_delete_honor_current_etag`; `if_match_requires_strong_tags_and_supports_existing_wildcard`; `mutation_if_none_match_rejects_current_representation_without_writing`; `transaction_if_none_match_rejects_current_representation_atomically`; `transaction_etag_precondition_is_atomic` | Boundary |
| HTTP-004 | GET と HEAD ルートが弱い比較、存在するリソースのワイルドカード、本文なしの 304、現在の ETag を伴う If-None-Match を尊重する。 | `src/server.rs`; `src/server/etag.rs`; `src/server/etag_tests.rs`; `docs/http-semantics.md` | `if_none_match_returns_not_modified_for_current_representation`; `head_reuses_record_headers_and_conditional_status` | Boundary |
| HTTP-005 | GET と HEAD のレコードルートがステータスと表現ヘッダーを共有し、HEAD は本文を送らず同じ条件付き ETag ステータスを使う。 | `src/server.rs`; `src/server/records.rs`; `src/server/etag_tests.rs`; `docs/http-semantics.md` | `head_reuses_record_headers_and_conditional_status` | Boundary |
| HTTP-006 | カタログスキーマの GET と HEAD ルートが強い表現 ETag と弱い If-None-Match による 304 を公開し、カタログトランザクションがカタログ write lock 内で If-Match と If-None-Match を評価し、条件失敗時は変更なしの 412 を返し、コミット後は新しい ETag を返す。 | `src/catalog.rs`; `src/catalog/transaction.rs`; `src/server/catalog.rs`; `src/server/catalog_transaction.rs`; `src/server/etag.rs`; `src/server/tests.rs`; `docs/ja/catalog.md`; `docs/ja/http-semantics.md` | `catalog_etag_guards_schema_reads_and_transactions` | Boundary |
| HTTP-007 | 単一テーブルと名前付きテーブルのストリームルートが、filter、projection、skip、limit だけを持つ JSON クエリ文書を受け付け、`Content-Length` なしで `application/x-ndjson` の一行一レコードを返し、有界スナップショット生成側による HTTP/1.1 chunked transfer を使う。 | `src/server/stream.rs`; `src/server.rs`; `src/server/catalog.rs`; `docs/ja/http-semantics.md` | `query_stream_endpoint_returns_chunked_ndjson`; `query_stream_endpoint_rejects_blocking_controls_before_streaming`; カタログストリームルートの確認 | Current |
| CI-001 | 英語と日本語の README 見出し構造を含むドキュメント翻訳の構造とリンク、フォーマット、lint、全ターゲットテストを、サポートする全 CI OS で必須にする。 | `scripts/check-doc-translations.sh`; `README.md`; `README.ja.md`; `.github/workflows/ci.yml` | `bash scripts/check-doc-translations.sh`; GitHub Actions のマトリクス実行 | Current |

## 明示的に残るギャップ

次の話題には文書または設計メモがありますが、マトリクスで現在の実装とは主張していません。

- 長寿命ストリーム向けのランタイム固有非同期トレイト。
- 完全なコストベースプランナー、完全なインデックス対応またはコストベースのマージ結合戦略、入力およびグループ出力の有界 `$match`、`$count`、`$distinct`、`$group`、グループ出力 `$project` を超える集約ステージ。
- カタログ全体の MVCC 可視性と過去の行バージョン。
- ロケール対応 CJK 照合と、より広い上流 CJK フィクスチャ。
- スキーマを保つ XBF から DBF へのエクスポートにおける厳密な複数ファイル読み取りアトミック性、オブジェクトストレージのマニフェスト、WASM ホスティング、分散レプリケーション。

これらのいずれかを Current にする前に、公開契約、壊れた入力の動作、クラッシュまたは再試行の動作、フィクスチャまたは決定的テスト、この表の行を追加します。

## 根拠となる資料

品質モデルは、SQLite の[テスト概要](https://sqlite.org/testing.html)、[品質管理計画](https://sqlite.org/qmplan.html)、[要件カタログ](https://sqlite.org/requirements.html)に記された分類に従います。

これらの資料は、追跡可能な要件、独立した壊れた入力と障害のテスト、再現可能なリリースゲートの根拠になります。

txBASE が SQLite と同じテスト量、カバレッジ、リリースプロセスを持つことを意味しません。
