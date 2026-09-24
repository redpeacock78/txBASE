# Worker Fetchオブジェクトストレージアダプター

この文書では、非同期オブジェクトストレージ契約向けのWeb Fetch APIアダプターを定義します。

実装は`src/worker-object-store.mjs`にあり、生成した`WasmObjectTable`ラッパーへ直接渡せます。

このアダプターは、`fetch`、`URL`、`Headers`、`AbortController`、Web Crypto、リクエストコンテキストのタイマーを提供するWorker形式のホストと互換性があります。

特定のクラウドベンダーをtxBASEがサポートするという主張ではなく、転送アダプターです。

## 1. アダプターの境界

`createWorkerObjectStore`は、`WasmObjectTable`が要求する5つのPromise返却メソッドを返します。

| メソッド | 結果 |
| --- | --- |
| `get(key)` | `Uint8Array`、またはオブジェクトが存在しない場合は`null` |
| `putIfAbsent(key, bytes)` | 不変な初回公開後に解決 |
| `compareAndSwap(key, expected, replacement)` | 条件付き置換後に解決 |
| `delete(key)` | 削除後に解決。存在しないオブジェクトは削除済みとして扱う |
| `list(prefix)` | ソート済みのオブジェクトキー文字列 |

アダプターは、ベースHTTP URL、任意の既定ヘッダー、任意の`AbortSignal`、0以上のリクエストタイムアウトを受け取ります。

既定のタイムアウトは30秒であり、既定のキャッシュモードは`no-store`です。

アダプターはリクエストを自動で再試行しません。

呼び出し側は共有する復旧規則と競合規則を確認してから、高レベル操作を再試行できます。

## 2. HTTPオブジェクトプロトコル

ベースURLはオブジェクトコレクションを示し、`http`または`https`を使わなければなりません。

アダプターは、スラッシュで区切ったオブジェクトキーの各要素をパーセントエンコードしたURLパス要素として追加します。

オブジェクトサービスは次の契約を実装しなければなりません。

| 操作 | リクエスト | 成功時 |
| --- | --- | --- |
| 読み取り | `GET /<key>` | オブジェクトバイト列を持つ`200`。`404`は`null` |
| 存在しない場合の書き込み | `If-None-Match: *`とバイト列のbodyを持つ`PUT /<key>` | `200`、`201`、または`204` |
| Compare-and-swap | 置換bodyと`If-None-Match: *`または`If-Match: <etag>`を持つ`PUT /<key>` | `200`、`201`、または`204` |
| 削除 | `DELETE /<key>` | `200`、`204`、または`404` |
| 列挙 | `GET /?prefix=<encoded prefix>` | 文字列配列のJSONを持つ`200` |

条件付き書き込みが一致しない場合、サービスは`409 Conflict`または`412 Precondition Failed`を返さなければなりません。

空でないcompare-and-swapのexpected値に対する`<etag>`は、expectedバイト列の強いSHA-256ダイジェストを引用符で囲み、パディングなしのbase64urlでエンコードした値です。

空のexpected値に対して、アダプターは`If-None-Match: *`を使います。

その他の`4xx`応答は不正なリクエストとして扱いますが、`408`と`429`は利用不能な転送結果として扱います。

すべての`5xx`応答は利用不能な結果として扱います。

オブジェクトキーは通常の空でないパス要素で構成し、`.`と`..`の要素はリクエスト送信前に拒否します。

列挙のprefixは末尾を`/`にして名前空間を選択できますが、それ以外の位置に空の要素を含めてはいけません。

## 3. タイムアウトとキャンセル

各操作は、fetchリクエスト用の専用`AbortController`を作成します。

設定したタイムアウトが満了するとリクエストを中断し、ホストの拒否値は`code: "unavailable"`になります。

呼び出し側の`AbortSignal`が中断されるとリクエストを中断し、ホストの拒否値は`code: "cancelled"`になります。

WASMブリッジは`invalid`、`conflict`、`missing`、`unavailable`、`cancelled`を対応する`ObjectStoreError`分類へ変換します。

アダプターは、リクエストの解決後に中断イベントリスナーとタイムアウトを解除します。

これにより、RustコアへWorker固有の依存を追加せずに、リクエストのキャンセルを`AsyncObjectTable`へ伝達できます。

## 4. WASMでの使用

```js
import { createWorkerObjectStore } from "./worker-object-store.mjs";

const store = createWorkerObjectStore({
  baseUrl: "https://objects.example.test/users/",
  headers: { Authorization: `Bearer ${env.TXBASE_TOKEN}` },
  signal: request.signal,
  timeoutMs: 10_000,
});
const table = new WasmObjectTable(store, "users");
```

オブジェクトサービスは、認証、リクエストルーティング、不変オブジェクトの作成、マニフェストの条件付き更新、強いETagの生成を担当します。

アダプターはXBFマニフェストと復旧プロトコルから独立しています。

## 5. 検証と残るホスト作業

`tests/wasm_worker_smoke.mjs`は、生成したWASMラッパーを決定的なローカルHTTPオブジェクトサービスへ接続します。

スモークテストは、バイト列の公開、マニフェストのcompare-and-swap、列挙、存在しない読み取り、競合の変換、タイムアウトの変換、WASMブリッジを通る呼び出し側キャンセルを検査します。

このテストはNodeのWeb Fetch APIをポータブルなホストフィクスチャとして使い、デプロイ済みのCloudflare WorkerまたはWASIランタイムを検査するものではありません。

WorkerまたはWASIのクエリストリームスケジューリング、転送のバックプレッシャー、非同期ローカルストレージ、ホストのライフサイクル動作は、引き続きホスト固有です。

クラウドベンダー固有の認証、整合性保証、保持、孤立オブジェクト削除のスケジューリング、サービス固有の再試行方針は、プロバイダーアダプターの文書に置きます。

## 一次資料と適用範囲

- [Cloudflare Workersのfetch API](https://developers.cloudflare.com/workers/runtime-apis/fetch/)
- [Cloudflare WorkersのWeb標準](https://developers.cloudflare.com/workers/runtime-apis/web-standards/)
- [Cloudflare WorkersのRequest `AbortSignal`](https://developers.cloudflare.com/workers/runtime-apis/request/)
- [Fetch Standard](https://fetch.spec.whatwg.org/)
- [DOM Standardの`AbortController`](https://dom.spec.whatwg.org/#interface-abortcontroller)
- [RFC 9110: HTTP Semantics](https://www.rfc-editor.org/rfc/rfc9110.html)

Cloudflareの資料はWorker FetchとWeb APIホストの語彙を定めます。

FetchとDOMの仕様は、転送とキャンセルのプリミティブを定めます。

RFC 9110は、アダプターが使う条件付きリクエストの意味論を定めます。

HTTPオブジェクトプロトコル、エラー対応付け、XBF統合はtxBASEが所有する契約です。
