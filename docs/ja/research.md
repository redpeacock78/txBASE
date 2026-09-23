# 仕様調査インデックス

このディレクトリには、txBASEの現在の境界と将来の作業を定義するために使った仕様を記録します。

文書は、次の3つの状態を区別します。

- **Current**。このリポジトリに動作が存在し、コードまたはテストでカバーされている。
- **Reference**。外部製品またはプロトコルを設計上の知見として調査している。
- **Future**。実装前に個別の契約が必要な提案である。

馴染みのある名前やJSON形状を使っただけで互換性を主張することはありません。

## トピック文書

| トピック | 文書 | 状態 |
| --- | --- | --- |
| dBASE と Visual FoxPro のファイル構造 | [DBF 互換性](dbf-compatibility.md) | 現在の対応と将来のエンコーディング作業 |
| クエリ文書、述語、カーソル、ストリーム | [クエリモデル](query-model.md) | 現在のサブセット |
| ランタイム非依存の非同期クエリストリーム | [非同期クエリストリーム](async-streaming.md) | 現在のポーリング境界とネイティブスレッドアダプター、将来のワーカーまたはWASIアダプター |
| 有界集約契約 | [集約モデル](aggregation.md) | 現在のサブセット |
| 有界ローカル結合契約 | [結合モデル](joins.md)と[カタログ](catalog.md) | 現在の境界 |
| MongoDB の述語とクエリ計画 | [クエリ計画](query-planning.md) | 現在のサブセットと参照資料 |
| 更新演算子とアトミック性 | [更新モデル](mutation-model.md) | 現在のサブセットと将来の境界 |
| ローカル変更データ取得 | [変更データ取得](change-data-capture.md) | 現在の単一テーブルと明示的な複数テーブルカタログのコミット済みイベント境界、有界な読み取り専用HTTP転送、将来の配信と再生 |
| スナップショットトランザクションと分離 | [スナップショットトランザクション](transactions.md)、[MVCC](mvcc.md)、[カタログ](catalog.md) | 現在のオプティミスティックなテーブル境界、テーブル集合の検証を伴う任意選択の粗粒度serializableテーブルおよびカタログロック、カタログHTTPの過去読み取り、将来の述語単位serializable作業 |
| Firestore と Realtime Database の設計 | [Firebase モデル](firebase-model.md) | 参照資料 |
| SQLite のテスト範囲と品質 | [テストと品質](testing-quality.md) | 現在のテストマップと参照資料 |
| 契約からテストへの追跡 | [品質契約マトリクス](quality-matrix.md) | 現在の証拠マップ |
| HTTP メソッド、PATCH、QUERY | [HTTP の意味](http-semantics.md) | 現在のルートとプロトコル参照 |
| DBF 上のリレーショナルスキーマ制約 | [スキーマメタデータ](schema-metadata.md) | 現在のローカルサブセットと将来のリレーショナル作業 |
| ネイティブ XBF ストレージ形式 | [XBF v1 草案](xbf.md) | 草案コーデック、DBF 変換と出力、スナップショット経路、世代検査付き WAL、スキーマ出力ジャーナル |
| 複数テーブル DBF 検出と有界ローカル等値結合 | [カタログ](catalog.md)と[結合モデル](joins.md) | 現在の境界と将来のリレーショナル作業 |
| 外部セカンダリインデックスのライフサイクル | [インデックス](indexes.md) | 現在のサイドカー保守、スカラーおよび複合キーの等値、複合等値プレフィックス候補、複合等値プレフィックス範囲候補、ヒストグラムによる範囲順序、順序プレフィックス、混在方向の複合プレフィックスソート、等値プレフィックス候補選択、一様統計による等値候補の順序付け、単独インデックスと積集合のコスト選択、レコード読み取り数、フィルター評価数、走査、ソート作業の決定的な行相当の説明フィールド、将来の I/O を考慮した完全なコストモデル |
| CJK、インデックス、XBF、ストレージ、並行性 | [ロードマップ](roadmap.md) | 現在の境界と将来の作業 |
| エッジとオブジェクトストレージのコミット | [エッジストレージ](edge-storage.md) | 現在のローカル境界と将来のクラウド作業 |
| WASMとワーカーのホスト境界 | [WASM](wasm.md) | ホスト非依存コアは現在、ホストアダプターは将来 |
| 分散レプリケーションと権威 | [分散化の進化](distributed-evolution.md) | 将来のアーキテクチャ |

## 調査方法

一次仕様とベンダー文書を優先します。

現在の動作として呼ぶ前に、実装とテストで確認します。

実装していない設計メモはFutureとしてラベル付けします。

このインデックスの調査は2026-09-22に更新しました。

READMEの構成は[texenv の README](https://github.com/redpeacock78/texenv/blob/master/README.md)の節構造に従いますが、内容はtxBASE固有です。

## 一次資料の監査

次の監査では、規範的な資料、製品文書、実装参照、プロジェクト固有の設計上の着想を分けます。

| 資料群 | 権威性 | txBASEとの整合 | 記載すべき場所 |
| --- | --- | --- | --- |
| dBASEとVisual FoxProのフォーマット資料 | dBASEのページはベンダー所有です。Visual FoxProのページはベンダーのヘルプを保存またはミラーしたものであり、現在の標準レジストリではありません。 | パーサー、memo読み取り、コードページ処理、フィールド幅規則は、フィクスチャでカバーする文書化済みサブセットだけを実装します。未対応形式はエラーのままであり、FoxPro全体の互換性は主張しません。 | フォーマットと互換性の詳細は[DBF互換性](dbf-compatibility.md)に置きます。ミラー資料の注意書きはこのインデックスに残し、互換性の主張ごとにフィクスチャを要求します。 |
| MongoDBマニュアル | 公式製品文書です。 | クエリ、インデックス、集約、結合の文書は語彙と一部の動作を借りますが、txBASE固有の有界性を追加します。MongoDBのwire、プランナー、パイプライン互換性は主張しません。 | 演算子の意味と比較語彙は、クエリ、集約、結合、インデックスの文書に残します。未実装のMongoDB動作を現在の契約へ取り込みません。 |
| Rust標準ライブラリのタスク文書 | 公式Rust API文書です。 | `AsyncQueryStream`境界は、executorを選択せず、ランタイム互換性を主張せずに`Context`、`Poll`、`Waker`、`Pin`のタスクモデルを再利用します。ネイティブの`ThreadedQueryStream`アダプターは、有界の標準ライブラリチャネルでこの契約を適用し、ワーカーまたはWASIのアダプターはホスト固有のままです。 | ポーリング契約とホストアダプターの責務は[非同期クエリストリーム](async-streaming.md)に置き、txBASEのクエリ意味論は[クエリモデル](query-model.md)に置きます。 |
| FirestoreとRealtime Databaseの文書 | 公式製品文書です。 | Firebase文書はアーキテクチャ参照だけです。txBASEはFirebaseのトランザクション、オフラインキュー、セキュリティルール、イベント同期を実装しません。 | 比較は[Firebaseモデル](firebase-model.md)に残し、DBF、HTTP、トランザクション契約へ持ち込みません。 |
| SQLite文書 | 公式プロジェクト文書です。 | 制約語彙、WALとアトミックコミットの根拠、クエリ計画語彙、テスト品質の実践を参照します。txBASEはDBFサイドカーと独自WALを使い、SQLiteのファイル、SQL、永続性互換性を主張しません。 | 根拠はスキーマ、MVCC、クエリ計画、テストの文書に置き、txBASEの動作は別に記述します。 |
| PostgreSQLのトランザクション分離とMVCC文書 | 公式プロジェクト文書です。 | トランザクション、MVCC、カタログの文書で、既定のオプティミスティックな古い元データの検査と、テーブル集合の検証を伴う任意選択の粗粒度serializableテーブルまたはカタログロックを、未実装の述語単位および分散serializable保証から区別するために使います。 | 区別は[スナップショットトランザクション](transactions.md)、[MVCC](mvcc.md)、[カタログ](catalog.md)に置き、PostgreSQLの分離保証を現在の契約へコピーしません。 |
| SQLiteセッション拡張とPostgreSQL論理デコード | 公式プロジェクト文書です。 | CDC文書は変更セットとコミット済みWAL利用者の語彙を参照しますが、単一テーブルcommit向けの`TXCD`と明示的な複数テーブルカタログcommit向けの`TXCC`という、物理レコードの状態差分サイドカーを定義します。利用者スロット、再生、レプリケーション互換性は提供しません。 | 外部資料との比較と範囲の境界は[変更データ取得](change-data-capture.md)に置き、DBFのcommit機構は[DBF互換性](dbf-compatibility.md)に、カタログジャーナル機構は[カタログ](catalog.md)に置きます。 |
| Gitのコマンドラインインターフェース文書 | CLI設計の参照に使う公式プロジェクト文書です。 | txBASEの明示的なサブコマンドとオプションの形だけに影響します。Gitのコマンド群、リポジトリモデル、オプションの意味はコピーしません。 | 設計判断は[CLIコマンド設計](cli.md)に置き、MVCCやストレージの契約には置きません。 |
| RFC 9110、RFC 5789、RFC 10008 | IETFの標準化過程にある仕様です。 | HTTPメソッドの安全性、PATCHの意味、QUERYの安全性と冪等性をHTTP契約へ反映します。txBASEは対応メディアタイプ、応答形状、範囲上限、ルート上限を別に定義します。 | 規範的なHTTP意味論は[HTTPの意味](http-semantics.md)に置き、txBASE固有の制約は実装契約のそばに置きます。 |
| POSIXの`rename()`と`fsync()` | The Open Groupの仕様です。 | Unixコードはrenameによる置換と`sync_all`を使います。Windowsには別の置換経路があるため、POSIXのディレクトリ永続性保証と同一とは記述しません。 | ファイルシステムの永続性の前提は永続化とXBFの文書に置き、Unix限定であることを明記します。 |
| WebAssembly、WASI、Component Model | 標準仕様または標準化中の仕様です。ホスト資料は実装参照です。 | リポジトリにはバージョン付きホスト非依存DBFコアと`wasm32-unknown-unknown`のコンパイル検査があります。ワーカーまたはWASIのランタイムアダプターと非同期ストレージは今後の作業です。 | コアABIとホスト境界の判断は[WASM](wasm.md)に置きます。ホスト固有の資料は、そのホストのアダプターとスモークテストを追加した時点で追記します。 |
| Raft論文とRaftプロジェクト資料 | 論文とプロジェクトの一次資料です。 | 分散化の進行で候補プロトコルを検討するために使いますが、txBASEがRaftを採用したことやレプリケーションを実装済みであることは意味しません。 | 候補の権威プロトコルは[分散化の進化](distributed-evolution.md)に置き、採用後のログと復旧契約を別途定義します。 |
| XBF v1形式草案 | このリポジトリが所有する仕様草案です。外部向けの互換性標準ではありません。 | 現在のコーデック、エッジ、世代検査付きWALのテストは草案を実装契約として検査します。外部リーダーでの原子的な公開が証明されるまでは草案のままです。 | バイト形式と受け入れ条件は[XBF v1草案](xbf.md)に置き、外部標準のようには扱いません。 |
| `encoding_rs` APIと固定した`dbf`互換性表 | 依存ライブラリのAPI文書と第三者実装の参照であり、エンコーディング標準ではありません。 | Rustのコーデック動作とlegacy aliasの特定に使います。互換性の根拠は第三者の表ではなく、固定したバイトフィクスチャです。 | [DBF互換性](dbf-compatibility.md)の実装補助資料として残し、規範的なフォーマット資料にはしません。 |
| texenv README | プロジェクト固有の文体上の着想であり、フォーマットまたはプロトコル資料ではありません。 | READMEの構成だけに影響し、txBASEの動作は依存しません。 | 帰属はこの調査インデックスに残し、機能契約には持ち込みません。 |

監査の結果、トピック文書へ2つの修正を反映しました。

1つ目は、`txbase schema apply`がアクティブレコードを検証し、サイドカーを原子的に置換するメタデータ専用編集コマンドであり、DBFレイアウト移行は今後の作業であることです。

2つ目は、POSIXの永続性に関する記述をUnix経路に限定し、クロスプラットフォームの置換動作は別に検査し、POSIXのディレクトリ永続性と同一とは宣伝しないことです。

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
- [Array query predicates](https://www.mongodb.com/docs/manual/reference/mql/query-predicates/arrays/)
- [`$all` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/all/)
- [`$elemMatch` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/elemmatch/)
- [`$size` query predicate](https://www.mongodb.com/docs/manual/reference/operator/query/size/)
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
- [MongoDB `$sum` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/sum/)
- [MongoDB `$avg` accumulator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/avg/)
- [MongoDB `$count` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/count/)
- [MongoDB `$project` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/project/)
- [MongoDB `$set` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/set/)
- [MongoDB `$addFields` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/addfields/)
- [MongoDB `$ifNull` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/ifnull/)
- [MongoDB `$literal` expression operator](https://www.mongodb.com/docs/manual/reference/operator/aggregation/literal/)
- [MongoDB `$unwind` aggregation stage](https://www.mongodb.com/docs/manual/reference/operator/aggregation/unwind/)
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

### トランザクションと並行性

- [PostgreSQLのトランザクション分離](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQLの並行性制御](https://www.postgresql.org/docs/current/mvcc.html)
- [PostgreSQLの論理デコード](https://www.postgresql.org/docs/current/logicaldecoding.html)
- [SQLiteの分離](https://sqlite.org/isolation.html)
- [SQLiteのWAL](https://sqlite.org/wal.html)
- [SQLiteセッション拡張](https://www.sqlite.org/sessionintro.html)

### HTTP

- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)
- [RFC 5789: PATCH Method](https://www.rfc-editor.org/rfc/rfc5789.html)
- [RFC 10008: The HTTP QUERY Method](https://www.rfc-editor.org/rfc/rfc10008.html)

### CLI設計

- [Gitのコマンドラインインターフェース規約](https://git-scm.com/docs/gitcli)

### Rustのタスクシステム

- [Rustの`Context`](https://doc.rust-lang.org/std/task/struct.Context.html)
- [Rustの`Poll`](https://doc.rust-lang.org/std/task/enum.Poll.html)
- [Rustの`Pin`](https://doc.rust-lang.org/std/pin/index.html)

### ファイルシステムのコミットプリミティブ

- [POSIX `rename()`](https://pubs.opengroup.org/onlinepubs/9799919799/functions/rename.html)
- [POSIX `fsync()`](https://pubs.opengroup.org/onlinepubs/009695399/functions/fsync.html)
- [POSIX file-system cache and directory durability rationale](https://pubs.opengroup.org/onlinepubs/9799919799/xrat/V4_xbd_chap01.html)

### WebAssemblyとホスト境界

- [WebAssemblyコア仕様](https://webassembly.github.io/spec/core/)
- [WASI](https://wasi.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [Cloudflare Workers WebAssembly](https://developers.cloudflare.com/workers/runtime-apis/webassembly/)
- [Node.js WASI](https://nodejs.org/api/wasi.html)

### 分散システム

- [In Search of an Understandable Consensus Algorithm（Raft）](https://raft.github.io/raft.pdf)
- [Raft consensus algorithm](https://raft.github.io/)

## レビュー規則

新しい機能が形式、クエリ、トランザクション、HTTPの境界をまたぐ場合は、該当するトピック文書を更新し、新しい契約を証明する最小のフィクスチャまたは失敗テストを追加します。

実装経路と再現可能な検査なしに、READMEへ広い互換性の主張を追加しません。
