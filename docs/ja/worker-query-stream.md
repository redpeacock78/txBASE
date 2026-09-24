# Workerクエリアダプター

この文書では、WASMクエリストリームをWebの`ReadableStream`として公開するJavaScriptホストアダプターを定義します。

これはWorkerと互換性のあるホスト境界です。
txBASEがデプロイ済みのCloudflare Worker統合を持つことは意味しません。

## 1. 境界

`WasmDatabase.query_stream_json()`は、既存の制限付きクエリパーサーと実行器を使ってスナップショット型クエリストリームを作成します。

ストリームはテーブルのスナップショットと検証済みのクエリ要求を所有します。
ストリーム作成後に`WasmDatabase`を更新しても、そのストリームから見えるレコードは変わりません。

`WasmQueryStream.next_json()`は、射影済みのレコードをJSON文字列として1件返し、ストリーム末尾では`null`を返します。
`WasmQueryStream.cancel()`は冪等であり、以後の取得をキャンセルエラーにします。

ストリームが受け付ける制御は、既存のfilter、projection、skip、limitだけです。
sort、aggregation、cursor、page制御は、バッファリングまたは別の再開契約が必要なため、ストリーム作成前に拒否します。

## 2. Web Streamsアダプター

`createWorkerQueryStream({ database, query, signal, queueSize })`は、UTF-8の`Uint8Array`をチャンクとして返す`ReadableStream`を作成します。
各チャンクはJSONレコード1件と`\n`で構成されるため、`application/x-ndjson`のレスポンスボディとして利用できます。

アダプターはpull型の基盤ソースを使います。
1回のpullでWASMストリームを最大1レコードだけ進め、1チャンクをキューへ追加します。
`queueSize`はWeb Streamsのキューに対する正の安全整数のハイウォーターマークで、既定値は`1`です。
結果配列全体を事前にマテリアライズしません。

Web Streamsのコンシューマーは、`read()`、`pipeTo()`、または`Response`のボディ消費によって需要を制御します。
コンシューマーが遅い場合も、基盤ソースは設定したキュー上限を超えてpullされません。

## 3. キャンセルとエラー

ホストは`AbortSignal`を指定できます。
AbortするとWASMストリームをキャンセルし、`WorkerQueryStreamError`のcodeが`cancelled`であるエラーにより`ReadableStream`を失敗させます。
リーダーの`cancel()`も同じキャンセル経路を使いますが、通常のコンシューマーキャンセルをデータエラーには変換しません。

不正なクエリ入力、対応しないストリーム制御、不正なキューサイズ、WASMメソッドの欠落はcodeが`invalid`です。
予期しないWASMエラーはcodeが`unavailable`です。
アダプターはリトライせず、クエリエラーを空のストリームとして扱いません。

このインメモリ実装では、キャンセルがライフサイクルの境界です。
ストリームはストレージI/Oを実行しないため、リモートストレージ要求はキャンセルしません。

## 4. 検証

`tests/wasm_query_stream_smoke.mjs`は生成済みの`wasm-bindgen`ラッパーを読み込み、次を検証します。

- 1件ずつのNDJSONチャンクが共通クエリ結果と一致すること。
- 後からデータベースを更新してもクエリのスナップショットが変わらないこと。
- キューサイズが未指定または0の場合に拒否すること。
- ストリーム作成前にsortを拒否すること。
- `AbortSignal`のキャンセルで保留中のreaderが`cancelled`カテゴリで失敗すること。
- WASMの直接キャンセルが観測可能で終端になること。

CIのWASMジョブは`wasm32-unknown-unknown`をビルドし、固定バージョンのNode.jsラッパーを生成し、既存のWASM smoke testとこのfixtureを実行します。

## 5. 範囲

アダプターはWebプラットフォームのストリームプリミティブを使うため、Worker形式のホストやNode.jsのWeb Streams fixtureに適用できます。
WASIスケジューラー、ネットワーククエリエンドポイント、リモートページ読み取り、カーソル再開、プロバイダー固有のストレージストリームは提供しません。

これらは別のホスト契約であり、それぞれI/O、タイムアウト、リトライ、backpressure、リカバリーの動作を定義します。

## 一次資料と範囲

- [WHATWG Streams Standard](https://streams.spec.whatwg.org/)
- [WHATWG DOM `AbortSignal`](https://dom.spec.whatwg.org/#interface-AbortSignal)
- [Cloudflare Workers Streams](https://developers.cloudflare.com/workers/runtime-apis/streams/)
- [Cloudflare Workers `ReadableStream`](https://developers.cloudflare.com/workers/runtime-apis/streams/readablestream/)
- [wasm-bindgenガイド](https://rustwasm.github.io/docs/wasm-bindgen/)

Streams Standardはpull型ソース、キュー、backpressure、reader、キャンセルを定義します。
DOM Standardは`AbortSignal`のライフサイクルを定義します。
Cloudflareの文書はWorker互換ホストの実装資料です。
これらの資料はtxBASEのクエリ意味論を定義せず、デプロイ済みWorkerのfixtureも保証しません。
