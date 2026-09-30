# WASMとワーカーホストの境界

この文書では、txBASEのコアとWebAssemblyホストの境界を定めます。

コアABIと現在のホストアダプターを説明します。
オブジェクトストレージ、ストリーミング、Worker、WASIの詳細は、それぞれの文書に記載します。

## 1. 現在のWASM API

`wasm::WasmCore`はインメモリの`DbfTable`を所有し、ネイティブと同じパーサー、クエリ、更新処理を再利用します。

共有する`DbfTable::apply_operation`の実装は、DBFトランザクション、WAL復旧、カタログのテーブル適用、WASMからも利用されます。

境界のバージョンは`ABI_VERSION = 1`です。

- `WasmCore::open_dbf`はDBFバイト列をインメモリ状態へ変換する。JavaScriptラッパーでは`new WasmDatabase(dbf)`がこの初期化を行う。
  `snapshot`は現在の状態をDBFバイト列として返す。
- `query_json`は有界なクエリ文書を実行する。
- `query_stream_json`は、filter、projection、skip、limit制御を受け付ける`WasmQueryStream`を返す。
- `WasmQueryStream.next_json()`は、呼び出すたびにJSON化した行を文字列で返し、ストリームの終端ではJavaScriptの`null`を返す。
- `WasmQueryStream.cancel()`を呼び出すと、その後の`next_json()`はキャンセルエラーを同期的に投げる。
- `apply_operation_json`は`POST`、`PUT`、`PATCH`、`DELETE`のいずれかの操作を適用する。
- `apply_operations_json`は空のバッチを拒否し、`{"operations":[...]}`バッチを非公開コピーへ適用して、全操作が成功した場合だけスナップショットを返す。

公開JSONメソッドは、デシリアライズ前に共有上限の1 MiBを超える`MAX_JSON_INPUT_BYTES`入力を拒否します。

共有する`MAX_OPERATION_BATCH`の上限は1,000操作です。

生成する`wasm-bindgen`の`WasmDatabase`ラッパーは、`wasm32-unknown-unknown`でコアAPIを公開します。

## 2. コアとホストの責務

`WasmCore`はインメモリのDBF状態を読み書きしますが、ホストのファイルアクセス、ネットワークリクエスト、タスクのスケジューリング、トランザクションの永続化は行いません。

コアはDBFとXBFのコーデック、クエリ意味論、更新規則、トランザクション状態遷移を担当します。

ホストは、返されたスナップショットの永続化、プラットフォームI/O、スケジューリング、並行性制御、ホスト固有のタイムアウトと再試行方針を担当します。

ホスト境界によって、コアの動作をPOSIXファイル、JavaScriptランタイム、特定バージョンのWASIへ依存させてはいけません。

## 3. 実装済みホストアダプター

- `AsyncObjectStore`と`AsyncObjectTable`は、ランタイム非依存のXBFオブジェクト操作、復旧、保持世代の読み取り、条件付き公開を提供する。
  生成する`WasmObjectTable`ラッパーは、これらの操作をPromiseを返すJavaScriptホストへ接続する。
  共有ストレージ契約は[エッジストレージ](edge-storage.md)に記載する。
- `createWorkerObjectStore`は、オブジェクトストレージ契約をWeb Fetchと条件付きHTTPリクエストへ対応付ける。
  `createR2ObjectStore`はCloudflare R2バインディングへ対応付ける。
  ホスト固有の動作は[Workerオブジェクトストレージ](worker-object-store.md)と[R2オブジェクトストレージ](r2-object-store.md)に記載する。
- `AsyncObjectTable::query_stream`と`query_stream_at`は、DBFバイト列を出力せずに、現在または保持中のXBFスナップショットから行をストリーミングする。
  生成ラッパーはこれらのクエリを`WasmObjectQueryStream`として公開し、`createWorkerQueryStream`は有界Web StreamsとNDJSONチャンクへ変換する。
- `AsyncObjectQueryStream`はランタイム非依存の`CancellationToken`をサポートするが、読み取りまたは一覧取得のfutureを破棄しても、ホストI/Oの停止までは保証しない。
  生成するWASM Promiseメソッドは、このRustトークンを公開しない。
  Workerのシグナル対応クエリは、クエリ単位の`AbortSignal`をホスト操作へ渡す。
  キャンセルの詳細は[非同期ストリーミング](async-streaming.md)と[Workerクエリストリーミング](worker-query-stream.md)に記載する。
- WASI 0.3 CLIコンポーネントは、書き込み可能な事前公開ファイルシステムを通じてDBFファイルまたは現行・保持世代のXBFスナップショットをクエリする。
  ホスト側で、このコマンドの実行中にほかのプロセスがオブジェクトストアへ書き込まないよう制御する。アダプターは同時書き込みを調整しない。
  ホストが書き込みを許可すれば、行の出力前に保留中WALを復旧する。
  ファイルシステム操作は同期処理である。
  コマンドとランタイムの契約は[WASIクエリストリーミング](wasi-query-stream.md)に記載する。

## 4. 検証範囲

CIは`wasm32-unknown-unknown`向けラッパーをビルドし、Node.jsで生成ラッパーをスモークテストします。

決定的なNode.jsフィクスチャは、Worker Fetch、R2バインディング、Web Streamsの各アダプターを検査します。

`wasi-query-stream`ジョブはWASI 0.3コンポーネントをビルドし、固定したWasmtimeランタイムでCLIをスモークテストします。

これらの検査対象は、生成バインディングとローカルホストフィクスチャです。
デプロイ済みWorker、実際のR2サービス、本番WASIホストとの互換性を示すものではありません。

[品質マトリクス](quality-matrix.md)に検査内容とテスト名を記載します。

## 5. 未対応のホスト保証

本番Workerへのデプロイと、実際のR2サービスへの接続検証は主張しません。

プロバイダー接続型のWASIストレージと、ノンブロッキングなWASIファイルシステムI/Oは未実装です。

ブラウザストレージ、Node.js WASI、Deno、Bunとの互換性も保証しません。

## 主な一次資料と適用範囲

- [`wasm-bindgen`ガイド：JavaScriptへエクスポートするRust型](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/exported-rust-types.html)
- [wasm-bindgen：PromiseとFuture](https://wasm-bindgen.github.io/wasm-bindgen/reference/js-promises-and-rust-futures.html)
- [wasm-bindgen：`Result<T, E>`](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/result.html)

`wasm-bindgen`ガイドは、エクスポートしたRust型がJavaScriptのクラスへ対応する規則と、async exportがPromiseへ変換される規則を説明します。
エクスポートした関数が返す`Result::Err`をJavaScript例外へ変換する規則も定めています。

txBASEのABIと入力上限は実装とテストで定めます。
ホスト互換性はアダプターとCIで検証した範囲に限ります。
WASI固有の資料は、targetとruntimeの境界を扱う[WASIクエリストリーミング](wasi-query-stream.md)に記載します。
