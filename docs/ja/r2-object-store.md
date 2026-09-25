# Cloudflare R2オブジェクトストレージアダプター

この文書は、Cloudflare WorkersのR2バケットバインディング向けアダプターを定義します。

実装は`src/r2-object-store.mjs`にあり、`WasmObjectTable`が必要とする5つのPromiseベースのメソッドを返します。

R2固有の条件付き書き込みとページ分割を定義します。
汎用Worker Fetchプロトコルは[Worker Fetchオブジェクトストレージアダプター](worker-object-store.md)に記載します。

## 1. アダプターの境界

`createR2ObjectStore(bucket)`はWorkersのR2バケットバインディングを受け取り、`get`、`putIfAbsent`、`compareAndSwap`、`delete`、`list`を返します。

| メソッド | 結果 |
| --- | --- |
| `get(key)` | `Uint8Array`。オブジェクトがなければ`null` |
| `putIfAbsent(key, bytes)` | キーが存在しない場合だけオブジェクトを公開 |
| `compareAndSwap(key, expected, replacement)` | 現在のバイト列が`expected`と一致する場合だけ置換 |
| `delete(key)` | オブジェクトを削除。存在しない場合も成功 |
| `list(prefix)` | 一致するすべてのR2ページから取得したキーを整列して返す |

アダプターは、R2バインディングの`get`、`put`、`delete`、`list`と、Workersが提供するWeb `Headers` APIを必要とします。

バインディング操作は自動で再試行しません。
タイムアウトやキャンセルの仕組みも追加しません。

## 2. 条件付き書き込み

`putIfAbsent`と、`expected === null`を指定したcompare-and-swapは、`If-None-Match: *`を付けて`put`を呼び出します。

`expected`が`null`でない場合、アダプターは現在のオブジェクトを読み取り、書き込み前にバイト列を比較します。

一致した場合は、そのオブジェクトの引用符付き`httpEtag`を`If-Match`に指定します。

この2つ目の条件により、読み取り後に発生した並行更新を上書きしません。

R2の生の`etag`をtxBASEのSHA-256ダイジェストとして扱いません。
アダプターは`httpEtag`をR2の条件付き書き込みトークンとしてだけ使います。

R2の条件付き`put`が`null`を返すと、アダプターは`code: "conflict"`で失敗します。

存在しないオブジェクトの`get`は`null`に対応付けます。
不正なバインディング結果は`invalid`に対応付け、バインディング操作の失敗は`unavailable`に対応付けます。

R2は読み取り、書き込み、削除、オブジェクト一覧について強整合性を提供すると説明しています。

## 3. 一覧とページ分割

アダプターは1ページにつき最大1,000件を要求し、`truncated`が`true`の間はR2の不透明な`cursor`を使って次のページを取得します。

R2は要求数より少ない件数を返すことがあるため、ページの件数だけでは終了を判定しません。

不正なページ、要求したプレフィックス外のキー、欠落または重複したカーソルは拒否します。

`AsyncObjectStore`は一致するすべてのキーを1つの`Vec`で返すため、アダプターは結果を整列する前に全ページをメモリへ蓄積します。

## 4. WASMでの使用

```js
import { WasmObjectTable } from "./txbase.js";
import { createR2ObjectStore } from "./r2-object-store.mjs";

const store = createR2ObjectStore(env.TXBASE);
const table = new WasmObjectTable(store, "users");
```

WorkerのWrangler設定で、R2バケットを`env.TXBASE`にバインドします。

テーブルのnamespaceはキーのプレフィックスを選びますが、バケットに対する認可境界ではありません。

Workerのルートで認可を行い、バインディングには必要なバケット操作だけを許可してください。

## 5. 検証と制限

`tests/wasm_r2_smoke.mjs`は、生成したWASMラッパーを決定的なインメモリR2バインディングフィクスチャへ接続します。

CIでは、スナップショットの公開と読み取り、存在しない読み取り、条件付き書き込みの競合、compare-and-swapの競合、削除の冪等性、複数ページの一覧、不正なカーソル、利用できないバインディングを検査します。

フィクスチャが検査するのはバインディングAPIの形です。
Workerをデプロイしたり、Cloudflareアカウントへ接続したりはしません。

アダプターが実装するのはオブジェクトストレージ操作だけです。
バケットのライフサイクルルールを設定せず、保持や孤立オブジェクトの削除をスケジュールせず、複数オブジェクト間のトランザクションも提供しません。

## 一次資料と適用範囲

- [Cloudflare R2 Workers APIリファレンス](https://developers.cloudflare.com/r2/api/workers/workers-api-reference/)
- [Cloudflare R2の整合性モデル](https://developers.cloudflare.com/r2/reference/consistency/)

Workers APIリファレンスは、ここで使うバインディング操作、条件付き操作、ETagフィールド、カーソルによる一覧取得を定義します。

整合性モデルは、R2操作に関するCloudflareの整合性保証を定義します。

アダプターの対応付けと制限はtxBASEの契約です。
ローカルフィクスチャではCloudflareの本番サービス動作を検証しません。
