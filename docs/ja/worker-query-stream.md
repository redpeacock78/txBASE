# Workerクエリアダプター

この文書では、WASMクエリストリームをWebの`ReadableStream`として公開するJavaScriptホストアダプターを定義します。

これはWorkerと互換性のあるホスト境界です。
txBASEがデプロイ済みのCloudflare Worker統合を持つことは意味しません。

## 1. 境界

`WasmDatabase.query_stream_json()`は、既存の制限付きクエリパーサーと実行器を使ってスナップショット型クエリストリームを作成します。

`WasmObjectTable.query_stream_json(body)`は、現在のコミット済みXBFスナップショットを復旧して読み込んだ後、JavaScriptの`Promise`を解決します。
`WasmObjectTable.query_stream_json_at(generation, body)`は、保持中の世代を1つ選択します。
両メソッドは所有型の`WasmObjectQueryStream`を返し、ランタイム非依存の非同期オブジェクトテーブルAPIと同じクエリパーサー、検証、XBFからDBFへの変換、行ストリームを使います。
DBFで表現できないXBFスナップショットは拒否します。

各ストリームはテーブルのスナップショットと検証済みのクエリ要求を所有します。
ストリーム作成後にどちらかのテーブルをcommitしても、そのストリームから見えるレコードは変わりません。

`WasmQueryStream.next_json()`と`WasmObjectQueryStream.next_json()`は、射影済みのレコードをJSON文字列として1件返し、ストリーム末尾では`null`を返します。
両方の`cancel()`は冪等であり、以後の取得をキャンセルエラーにします。

ストリームが受け付ける制御は、既存のfilter、projection、skip、limitだけです。
sort、aggregation、cursor、page制御は、バッファリングまたは別の再開契約が必要なため、ストリーム作成前に拒否します。

## 2. Web Streamsアダプター

`createWorkerQueryStream({ database, query, generation, signal, queueSize })`は、UTF-8の`Uint8Array`をチャンクとして返す`ReadableStream`を作成します。
各チャンクはJSONレコード1件と`\n`で構成されるため、`application/x-ndjson`のレスポンスボディとして利用できます。
`generation`を省略すると`database.query_stream_json(body)`を呼びます。
0以上の符号なし64ビット`BigInt`を指定すると、`database.query_stream_json_at(generation, body)`を呼びます。

アダプターはpull型の基盤ソースを使います。
必要な場合はPromiseを返すストリーム生成を待ち、その後1回のpullでWASMストリームを最大1レコードだけ進め、1チャンクをキューへ追加します。
`queueSize`はWeb Streamsのキューに対する正の安全整数のハイウォーターマークで、既定値は`1`です。
結果配列全体を事前にマテリアライズしません。
オブジェクトストアのクエリでは、最初の行を返す前にコミット済みXBFスナップショット全体を読み込み、DBFへ変換します。
行の配送はストリーミングしますが、リモートスナップショットをページ単位で読み込むわけではありません。

Web Streamsのコンシューマーは、`read()`、`pipeTo()`、または`Response`のボディ消費によって需要を制御します。
コンシューマーが遅い場合も、基盤ソースは設定したキュー上限を超えてpullされません。

## 3. キャンセルとエラー

ホストは`AbortSignal`を指定できます。
AbortするとWASMストリームをキャンセルし、`WorkerQueryStreamError`のcodeが`cancelled`であるエラーにより`ReadableStream`を失敗させます。
シグナル対応のWASMオブジェクトテーブルメソッドを使うと、Workerアダプターはクエリごとのシグナルをスナップショット読み込み中のFetchリクエストにも渡します。
そのクエリをキャンセルすると対応するリクエストだけが中断され、並行する別のクエリには影響しません。
ランタイム非依存の`AsyncObjectStore`契約には操作単位のキャンセルトークンがありません。
その他のアダプターとホストは、独自にI/Oキャンセル方針を定義する必要があります。
リーダーの`cancel()`も同じキャンセル経路を使いますが、通常のコンシューマーキャンセルをデータエラーには変換しません。

不正なクエリ入力、対応しないストリーム制御、不正な世代またはキューサイズ、WASMメソッドの欠落はcodeが`invalid`です。
Promiseを返すオブジェクトテーブルの初期化エラーは、最初の行を返す前にストリームエラーとして通知します。
予期しないWASMエラーはcodeが`unavailable`です。
アダプターはリトライせず、クエリエラーを空のストリームとして扱いません。

ストレージI/Oを実行しないインメモリ実装では、キャンセルはライフサイクルの境界です。
リモートスナップショットI/Oをキャンセルするのは、前述したWorkerのシグナル対応オブジェクトテーブル経路だけです。

## 4. 検証

`tests/wasm_query_stream_smoke.mjs`は生成済みの`wasm-bindgen`ラッパーを読み込み、次を検証します。

- 1件ずつのNDJSONチャンクが共通クエリ結果と一致すること。
- 後からデータベースを更新してもクエリのスナップショットが変わらないこと。
- Promiseを返すオブジェクトテーブルが、現在世代と保持世代の行を返すこと。
- Promiseによる初期化中のキャンセルで行を返さず、初期化後にストリームをキャンセルすること。
- キューサイズが未指定または0の場合に拒否すること。
- ストリーム作成前にsortを拒否すること。
- `AbortSignal`のキャンセルで保留中のreaderが`cancelled`カテゴリで失敗すること。
- WASMの直接キャンセルが観測可能で終端になること。

`tests/wasm_worker_smoke.mjs`は、クエリのキャンセルが実行中のスナップショットFetchを中断し、並行する別のクエリには個別にキャンセルするまで影響しないことを検証します。

CIのWASMジョブは`wasm32-unknown-unknown`をビルドし、固定バージョンのNode.jsラッパーを生成し、既存のWASM smoke testとこのfixtureを実行します。

## 5. 範囲

アダプターはWebプラットフォームのストリームプリミティブを使うため、Worker形式のホストやNode.jsのWeb Streams fixtureに適用できます。
WASIスケジューラー、ネットワーククエリエンドポイント、リモートページ読み取り、カーソル再開、WorkerのFetch経路を超える汎用`AsyncObjectStore`キャンセル契約は提供しません。

これらは別のホスト契約であり、それぞれI/O、タイムアウト、リトライ、backpressure、リカバリーの動作を定義します。

## 一次資料と範囲

- [WHATWG Streams Standard](https://streams.spec.whatwg.org/)
- [WHATWG DOM `AbortSignal`](https://dom.spec.whatwg.org/#interface-AbortSignal)
- [Cloudflare Workers Streams](https://developers.cloudflare.com/workers/runtime-apis/streams/)
- [Cloudflare Workers `ReadableStream`](https://developers.cloudflare.com/workers/runtime-apis/streams/readablestream/)
- [wasm-bindgen：PromiseとFuture](https://wasm-bindgen.github.io/wasm-bindgen/reference/js-promises-and-rust-futures.html)
- [wasm-bindgen：exportするRust型](https://wasm-bindgen.github.io/wasm-bindgen/reference/types/exported-rust-types.html)

Streams Standardはpull型ソース、非同期pull処理、キュー、backpressure、reader、キャンセルを定義します。
DOM Standardは`AbortSignal`のライフサイクルを定義します。
`wasm-bindgen`ガイドは、exportしたRustの`async fn`がJavaScriptの`Promise`になり、export型をその戻り値に使えることを定義します。
Cloudflareの文書はWorker互換ホストの実装資料です。
これらの資料はtxBASEのクエリ意味論やストレージFutureのキャンセルを定義せず、デプロイ済みWorkerのfixtureも保証しません。
