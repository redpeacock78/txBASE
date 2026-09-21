# 仕様調査インデックス

このディレクトリには、txBASE の現在の境界と将来の作業を定義するために使った仕様を記録します。

文書は、次の三つの状態を区別します。

- **Current**：このリポジトリに動作が存在し、コードまたはテストでカバーされている。
- **Reference**：外部製品またはプロトコルを設計上の知見として調査している。
- **Future**：実装前に個別の契約が必要な提案である。

馴染みのある名前や JSON 形状を使っただけで互換性を主張することはありません。

## トピック文書

| トピック | 文書 | 状態 |
| --- | --- | --- |
| dBASE と Visual FoxPro のファイル構造 | [DBF 互換性](dbf-compatibility.md) | 現在の対応と将来のエンコーディング作業 |
| クエリ文書、述語、カーソル、ストリーム | [クエリモデル](query-model.md) | 現在のサブセット |
| 有界集約契約 | [集約モデル](aggregation.md) | 現在のサブセット |
| 有界ローカル結合契約 | [結合モデル](joins.md) と [カタログ](catalog.md) | 現在の境界 |
| MongoDB の述語とクエリ計画 | [クエリ計画](query-planning.md) | 現在のサブセットと参照資料 |
| 更新演算子とアトミック性 | [更新モデル](mutation-model.md) | 現在のサブセットと将来の境界 |
| Firestore と Realtime Database の設計 | [Firebase モデル](firebase-model.md) | 参照資料 |
| SQLite のテスト範囲と品質 | [テストと品質](testing-quality.md) | 現在のテストマップと参照資料 |
| 契約からテストへの追跡 | [品質契約マトリクス](quality-matrix.md) | 現在の証拠マップ |
| HTTP メソッド、PATCH、QUERY | [HTTP の意味](http-semantics.md) | 現在のルートとプロトコル参照 |
| DBF 上のリレーショナルスキーマ制約 | [スキーマメタデータ](schema-metadata.md) | 現在のローカルサブセットと将来のリレーショナル作業 |
| ネイティブ XBF ストレージ形式 | [XBF v1 草案](xbf.md) | 草案コーデック、DBF 変換と出力、スナップショット経路、世代検査付き WAL、スキーマ出力ジャーナル |
| 複数テーブル DBF 検出と有界ローカル等値結合 | [カタログ](catalog.md) と [結合モデル](joins.md) | 現在の境界と将来のリレーショナル作業 |
| 外部セカンダリインデックスのライフサイクル | [インデックス](indexes.md) | 現在のサイドカー保守、等値、複合等値プレフィックス範囲候補、ヒストグラムによる範囲順序、順序プレフィックス、混在方向の複合プレフィックスソート、等値プレフィックス候補選択、一様統計による等値候補の順序付け、単独インデックスと積集合のコスト選択、有界なレコード数、走査、ソートのコスト選択、将来の I/O を考慮した完全なコストモデル |
| CJK、インデックス、XBF、ストレージ、並行性 | [ロードマップ](roadmap.md) | 現在の境界と将来の作業 |

## 調査方法

一次仕様とベンダー文書を優先します。

現在の動作として呼ぶ前に、実装とテストで確認します。

実装していない設計メモは Future としてラベル付けします。

このインデックスの調査は 2026-09-21 に更新しました。

README の構成は [texenv の README](https://github.com/redpeacock78/texenv/blob/master/README.md)の節構造に従いますが、内容は txBASE 固有です。

## 主な資料群

### dBASE と Visual FoxPro

- [dBASE Level 7 file format](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)
- [Visual FoxPro table file structure](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm)
- [Visual FoxPro variable-length fields](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm)
- [Visual FoxPro memo file structure](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm)
- [Visual FoxPro auto-increment fields](https://www.vfphelp.com/vfp9/html/bd6eff0c-2ce5-43b7-ab29-f5360cd2f90e.htm)
- [Visual FoxPro code pages](https://www.vfphelp.com/help/html/a3d7b0e0-8320-44b1-8983-17c30a78c6c4.htm)

### MongoDB

- [Documents](https://www.mongodb.com/docs/manual/core/document/)
- [Query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/)
- [Find command](https://www.mongodb.com/docs/manual/reference/command/find/)
- [`$expr` field expressions](https://www.mongodb.com/docs/manual/reference/operator/query/expr/)
- [Query optimization](https://www.mongodb.com/docs/manual/core/query-optimization/)
- [Explain and execution statistics](https://www.mongodb.com/docs/manual/reference/method/db.collection.explain/)
- [MongoDB compound indexes](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/)
- [BSON comparison order](https://www.mongodb.com/docs/manual/reference/bson-type-comparison-order/)
- [Compound-index sort order](https://www.mongodb.com/docs/manual/core/indexes/index-types/index-compound/sort-order/)
- [Equality-sort-range guideline](https://www.mongodb.com/docs/manual/tutorial/equality-sort-range-guideline/)
- [Update operators](https://www.mongodb.com/docs/manual/reference/mql/update/)
- [Atomicity and transactions](https://www.mongodb.com/docs/manual/core/write-operations-atomicity/)
- [MongoDB cursors](https://www.mongodb.com/docs/manual/core/cursors/)
- [MongoDB `$group` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/group/)
- [MongoDB `$count` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
- [MongoDB `$lookup` join stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/lookup/)

### Firebase

- [Firestore data model](https://firebase.google.com/docs/firestore/data-model)
- [Firestore transactions and batched writes](https://firebase.google.com/docs/firestore/manage-data/transactions)
- [Firestore transaction contention and serializable isolation](https://firebase.google.com/docs/firestore/transaction-data-contention)
- [Firestore write-time aggregation](https://firebase.google.com/docs/firestore/solutions/aggregation)
- [Realtime Database save data](https://firebase.google.com/docs/database/admin/save-data)
- [Realtime Database security](https://firebase.google.com/docs/database/security)
- [Realtime Database offline capabilities](https://firebase.google.com/docs/database/android/offline-capabilities)
- [Firestore query cursors](https://firebase.google.com/docs/firestore/query-data/query-cursors)

### SQLite

- [How SQLite Is Tested](https://sqlite.org/testing.html)
- [Atomic Commit In SQLite](https://sqlite.org/atomiccommit.html)
- [Write-Ahead Logging](https://sqlite.org/wal.html)
- [SQLite database file format](https://sqlite.org/fileformat.html)
- [SQLite query planning](https://sqlite.org/queryplanner.html)
- [SQLite ANALYZE](https://www.sqlite.org/lang_analyze.html)
- [SQLite requirements](https://sqlite.org/requirements.html)
- [SQLite quality management](https://sqlite.org/qmplan.html)
- [SQLite TH3](https://sqlite.org/th3.html)
- [SQLite limits](https://sqlite.org/limits.html)
- [SQLite `CREATE TABLE` constraints](https://sqlite.org/lang_createtable.html)
- [SQLite foreign-key support](https://www.sqlite.org/foreignkeys.html)

### HTTP

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)

### ファイルシステムのコミットプリミティブ

- [POSIX `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)
- [POSIX `fsync()`](https://pubs.opengroup.org/onlinepubs/009695399/functions/fsync.html)
- [POSIX file-system cache and directory durability rationale](https://pubs.opengroup.org/onlinepubs/9799919799/xrat/V4_xbd_chap01.html)

## レビュー規則

新しい機能が形式、クエリ、トランザクション、HTTP の境界をまたぐ場合は、該当するトピック文書を更新し、新しい契約を証明する最小のフィクスチャまたは失敗テストを追加します。

実装経路と再現可能な検査なしに、README へ広い互換性の主張を追加しません。
