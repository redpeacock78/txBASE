# WASMとワーカーのホスト境界

この文書では、WASMとエッジランタイムの境界を分離して記述します。

リポジトリには、ホスト非依存DBFコアのスライス、ランタイム非依存の非同期オブジェクトストレージ境界、非同期XBFオブジェクトテーブルのcommit用JavaScriptホストアダプターがあります。
明示的なタイムアウトとキャンセルの対応付けを持つ、Worker互換のFetch転送アダプターもあります。
有界pullスケジューリングと`AbortSignal`キャンセルを持つWorker互換のWeb Streamsクエリアダプターもあります。
WASI固有のクエリストリームランタイムアダプターは今後の作業です。

## 0. 現在の実装スライス

`wasm::WasmCore`はインメモリの`DbfTable`を所有し、ネイティブのパーサー、クエリ検証、クエリ実行、更新メソッドを再利用します。

共有する`DbfTable::apply_operation`の実装は`src/dbf/operation.rs`が所有し、DBFトランザクション、WAL復旧、カタログのテーブル操作、WASMから利用します。
WASMはパスとbodyの検証を二重に実装しません。

現在のバージョン付き境界は次を提供します。

- `ABI_VERSION = 1`。
- すべての公開JSONメソッドは、デシリアライズ前に共有する`MAX_JSON_INPUT_BYTES`（現在は1 MiB）を超える入力を拒否する。
- DBFバイト列を入出力する`open_dbf`と`snapshot`。
- 既存の有界クエリ文書を受け取る`query_json`。
- 既存のfilter、projection、skip、limit制御を受け取り、pullごとにJSONレコードを1件返す、スナップショット所有型の`query_stream_json`。
- 既存の`POST`、`PUT`、`PATCH`、`DELETE`操作IRを受け取る`apply_operation_json`。
- 同じ操作IRを有界な`{"operations":[...]}`トランザクション文書で受け取る`apply_operations_json`。
  すべての操作が成功した場合だけ、非公開コピーからスナップショットを公開する。
  共有する`MAX_OPERATION_BATCH`の上限は1,000操作であり、上限超過または空のバッチは操作を適用する前に拒否する。
- `wasm32`で同じメソッドを公開する`wasm-bindgen`の`WasmDatabase`ラッパー。
- ネイティブ契約テスト。
- 生成したラッパーを読み込み、ABIバージョンとスナップショットの往復を検査し、4種類の更新操作と原子的なバッチのロールバックを検査する、固定したNode.js `wasm-bindgen`スモークテスト。
- JavaScriptのオブジェクトストレージホストを受け取り、Promiseを返す`get`、`putIfAbsent`、`compareAndSwap`、`delete`、`list`を`AsyncObjectStore`へ接続し、XBFの読み取り、commit、復旧、過去世代読み取り、保持、孤立オブジェクト削除を公開する`WasmObjectTable`アダプター。
- それら5つの操作をWeb Fetch、条件付きリクエスト、強いSHA-256 ETag、リクエストタイムアウト、`AbortSignal`によるキャンセルでHTTPオブジェクトサービスへ対応付ける`createWorkerObjectStore`アダプター。
- compare-and-swap公開、WALクリーンアップ失敗後の復旧、過去世代読み取り、保持、ホストエラー変換を検査する固定Node.jsホストフィクスチャ。
- 生成したWASMラッパーを介してWorker転送を検査し、タイムアウトとキャンセルを含めてWeb Fetchを検証する固定Node.jsフィクスチャ。
- WASMのスナップショットストリームを、有界なNDJSONチャンクのWeb `ReadableStream`として公開し、readerのキャンセルと`AbortSignal`のライフサイクルを処理する`createWorkerQueryStream`アダプター。
- `AsyncObjectStore`を通じて現在または保持中のコミット済みXBFスナップショットを1つ読み取り、XBFからDBFへ変換した後、既存のfilter、projection、skip、limitのクエリストリーム意味論を再利用する、ランタイム非依存の`AsyncObjectTable::query_stream`と`query_stream_at`アダプター。
- backpressureを考慮したpullスケジューリング、スナップショットの安定性、不正な制御、キャンセルを検査する固定Node.js Web Streamsフィクスチャ。
- CIでの`wasm32-unknown-unknown` release buildとラッパースモーク検査。

コアはファイル書き込み、ネットワークアクセス、タスクのスケジューリング、トランザクションのコミットを行いません。
ホストは、返されたスナップショットの永続化、直列化、再試行、並行性制御を提供しなければなりません。

## 1. コアの境界

txBASEのコアは、決定的でホストから独立した構造を保ちます。

ホストは、HTTP、非同期ストレージ、時計、プラットフォーム固有APIを提供します。

コアは、形式コーデック、クエリ意味論、更新規則、WALエンコード、トランザクション状態遷移を担当します。

## 2. ホストとコアの分離

候補となる構成は次のとおりです。

```text
JavaScriptまたはTypeScriptのホスト
        |
        ├─ HTTP
        ├─ 非同期オブジェクトストレージ
        ├─ プラットフォームAPI
        ↓
     txbase.wasm
        |
        ├─ DBFとXBFのコーデック
        ├─ クエリエンジン
        ├─ 更新意味論
        ├─ WALコーデック
        └─ トランザクション状態機械
```

ホストは、コアをPOSIXファイルやWASI固有の動作へ依存させてはいけません。

## 3. 再利用する契約

WASM境界は、対応する範囲で既存のDBFとXBFのコーデックを再利用します。

ネイティブライブラリと同じ有界クエリ、更新、検証、復旧の意味論を公開します。

2つ目のクエリ言語、2つ目のトランザクションモデル、ホスト固有のDBFバイト解釈は導入しません。

ランタイムから独立した`AsyncQueryStream`契約は、executorを選択せずに、ホストのポーラーへ同じクエリストリームの項目意味論を提供します。

メモリ内実装は即時に完了します。ワーカーまたはWASIホストは、スケジューリング、起床、バックプレッシャー、タイムアウト、キャンセル、転送の動作を別に提供しなければなりません。

ネイティブの`ThreadedQueryStream`アダプターは、`wasm32`の外側で有界スケジューリング、起床、バックプレッシャー、破棄時キャンセルを提供します。

これは共有契約のネイティブホスト実装であり、WASM ABIを変更せず、ワーカーまたはWASIのホストサービスも提供しません。

Worker互換の`createWorkerQueryStream`アダプターは、Rustのexecutorを使わずにWeb Streamsのpull境界を利用します。
1回のpullでWASMスナップショットを最大1レコードだけ進め、正のキューのハイウォーターマークを適用し、UTF-8のNDJSONチャンクを生成し、readerまたは`AbortSignal`のキャンセルをWASMストリームのライフサイクルへ対応付けます。

オブジェクトストレージの契約は、共有するテーブルとトランザクションのインターフェースより下位に置きます。
ランタイムから独立した`AsyncObjectStore`契約は、5つの基本オブジェクト操作について実装済みです。
`AsyncObjectTable`は、その操作を通じてマニフェスト、世代、復旧、保持、条件付き公開の契約を再利用します。
どちらの境界もexecutorを選択せず、ブロッキングなファイルシステム呼び出しを非ブロッキングにも変換しません。

`wasm-bindgen`のJavaScriptアダプターは、Promiseを返すホストオブジェクトに同じ5つの操作を委譲します。
`get`は`Uint8Array`または`null`を、`list`は文字列キーを、その他の操作は`undefined`を解決します。
ホストの拒否オブジェクトは`invalid`、`conflict`、`missing`、`unavailable`、`cancelled`の`code`を指定でき、アダプターは共有する`ObjectStoreError`の分類へ変換します。
codeのない拒否は`unavailable`になります。
Worker FetchアダプターはHTTP転送、タイムアウト、キャンセルの対応付けを提供し、再試行方針はホスト側とプロバイダー側が担当します。

## 4. 対象ホスト

同じコアは、将来的に次のホストの背後で実行できます。

- ネイティブRust API。
- Cloudflare Workersなどのワーカーランタイム。
- OPFSなどのブラウザストレージ。
- Node、Deno、Bunのアダプター。
- WASI互換ランタイム。
- 必要なストレージプリミティブを提供する別のホスト。

この一覧は互換性の目標であり、すべてのホストをサポートする約束ではありません。

## 5. ホストサービス

実装前に、境界で次の事項を定義しなければなりません。

- 非同期の範囲読み取りと書き込み。
- 不変オブジェクトの公開。
- 条件付きマニフェスト更新。
- メモリとペイロードの上限。
- キャンセルとタイムアウトの動作。
- エラーと再試行の対応付け。
- テスト用の決定的な時計または世代入力。

ホストはスケジューリングとプラットフォームI/Oを担当します。

コアはコミット済みデータベース状態の意味を担当します。

## 6. 完了条件

現在のコアスライスは、最初の条件として次を満たします。

- バージョン付きホスト非依存ABIがある。
- ネイティブホストのフィクスチャが1つある。
- ネイティブ経路とWASM経路が同じクエリと更新の実装経路を使う。
- ランタイムから独立したストレージ契約を使う非同期テーブルフィクスチャが1つある。
- 生成した`wasm-bindgen`ラッパーを使うJavaScriptホスト接続型の非同期オブジェクトテーブルフィクスチャが1つある。
- Worker互換のFetchオブジェクトストレージアダプターと決定的なHTTPフィクスチャがある。
- Worker互換のWeb Streamsクエリアダプターと決定的なNode.jsフィクスチャがある。
- 現在または保持中のDBFで表現できるXBFスナップショット向けに、ランタイム非依存の非同期ストレージ接続型クエリストリームアダプターがある。
- バイト列とJSONの境界で不正入力エラーを明示的に扱う。
- 生成した`wasm-bindgen`ラッパーをNode.jsから検査するスモークテストがある。

デプロイ済みワーカーまたはWASIホストを完了と呼ぶ前に、次の条件を満たします。

- 選択したワーカーまたはWASIランタイムのスモークテストが1つある。
- WASI固有のクエリストリームスケジューラー、バックプレッシャー、ライフサイクル契約がある。
- 非同期クエリストリーム向けのホスト固有のタイムアウト、再試行、キャンセル動作がある。
- リモートストレージを選択する場合、プロバイダー固有の整合性と再試行の契約がある。

それまでは、Worker Fetch転送を現在の汎用ホスト境界として扱い、デプロイ済みワーカーまたはWASIランタイム統合を今後の作業とします。

## 7. 明示的な非目標

この文書は、JavaScript ORM、ブラウザー専用データベース、またはWASM内部のPOSIXエミュレーション層を約束しません。

これらは別の製品となり、共有コアの契約を不明確にします。

## 一次資料と適用範囲

- [WebAssemblyコア仕様](https://webassembly.github.io/spec/core/)
- [WASI](https://wasi.dev/)
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
- [Cloudflare Workers WebAssembly](https://developers.cloudflare.com/workers/runtime-apis/webassembly/)
- [Cloudflare Workersのfetch API](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [Cloudflare WorkersのWeb標準](https://developers.cloudflare.com/workers/runtime-apis/web-standards/)
- [Cloudflare WorkersのRequest `AbortSignal`](https://developers.cloudflare.com/workers/runtime-apis/request/)
- [Node.js WASI](https://nodejs.org/api/wasi.html)
- [wasm-bindgenガイド](https://rustwasm.github.io/docs/wasm-bindgen/)
- [`wasm-bindgen-futures` API](https://docs.rs/wasm-bindgen-futures/latest/wasm_bindgen_futures/)
- [`js-sys`の`Function::apply` API](https://docs.rs/js-sys/latest/js_sys/struct.Function.html)

WebAssemblyとWASIの仕様は、コアモジュールとホストインターフェースの語彙を定義します。
Component Model、Cloudflare Workers、Node.jsの資料は候補ホストの実装参照であり、txBASEの互換性を約束するものではありません。

リポジトリにはWASMコアの実装、生成ラッパーのNode.jsスモーク検査、JavaScriptホスト接続型の非同期オブジェクトテーブルフィクスチャ、Worker互換Fetchオブジェクトストレージアダプターとスモークフィクスチャ、ランタイムから独立した非同期オブジェクトストレージとテーブルの契約があります。
DBFで表現できるXBFスナップショット向けのランタイム非依存非同期ストレージ接続型クエリストリームアダプター、Worker互換Web Streamsクエリアダプター、スモークフィクスチャもあります。
デプロイ済みワーカーまたはWASIランタイム、WASI固有のクエリストリームスケジューラー、プロバイダー固有の整合性または再試行方針、ネイティブの復旧経路をすでにサポートするとは主張しません。
