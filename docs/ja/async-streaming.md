# 非同期クエリストリーム

この文書は、長寿命クエリストリームのランタイム非依存なポーリング境界を定義します。

executor、ワーカーランタイム、ネットワークプロトコル、ストレージサービスは選択しません。

## 1. ポーリング契約

`AsyncQueryStream`は、Rustのタスクシステムと同じ3状態の結果を持つ`poll_next`を公開します。

| 結果 | 意味 |
| --- | --- |
| `Poll::Ready(Some(item))` | クエリ結果を1件利用できます。 |
| `Poll::Ready(None)` | ストリームが終了し、以後は項目を返しません。 |
| `Poll::Pending` | ストリームがまだ準備できていません。次のポーリング前に、渡されたwakerを呼び出す必要があります。 |

呼び出し側は、各ポーリングのためにストリームをpinします。

ストリームの破棄が、実装が所有するリソースのキャンセル境界です。

このトレイトは、ワーカーとWASMのホストがローカルタスクを使えるように、`Send`、`Sync`、特定のexecutorを要求しません。

実装は`poll_next`の内部でブロッキング操作を実行してはなりません。

## 2. 現在の実装

`QueryStream`と`QuerySnapshotStream`は`AsyncQueryStream`を実装します。

どちらもメモリ内テーブルを読むため、ポーリングはタスクコンテキストを使わず、`Ready`で即時に完了します。

`QueryStream`は元のテーブルを借用します。

`QuerySnapshotStream`はテーブルの複製を所有し、元のテーブルが変更された後も結果を安定させます。

`BoundedQueryStream`は、標準ライブラリのチャネルを使うブロッキングイテレータとして残ります。

`poll_next`から`recv`を呼ぶとホストのタスクをブロックするため、`AsyncQueryStream`は実装しません。

ネイティブターゲットでは、`stream_query_threaded`が`ThreadedQueryStream`を返します。

これはテーブルを複製し、既存のスナップショットストリームを1本のワーカースレッドで実行し、正の容量を持つ標準ライブラリチャネルで結果を公開します。

`poll_next`は`try_recv`だけを使い、チャネルが空のときに呼び出し側のwakerを登録し、登録後にチャネルを再確認してwakerの取りこぼしを防ぎます。

チャネルが満杯になると、ワーカーにバックプレッシャーがかかります。

ストリームを破棄すると、キャンセルフラグを立て、receiverを閉じ、ワーカーをjoinするため、ネイティブの生成処理がクエリの所有者より長く残りません。

このアダプターは`wasm32`向けにはコンパイルされません。

この制約は`ThreadedQueryStream`だけに適用されます。
別のWASIクエリストリームコンポーネントは、共有するインメモリの`QueryStream`をポーリングし、行をWASIの非同期stdoutへ渡します。
コマンドとランタイムの境界は[WASIクエリストリーム](wasi-query-stream.md)で説明します。

`AsyncObjectTable::query_stream`は、現在のコミット済みスナップショット向けに`AsyncObjectQueryStream`を返します。

`AsyncObjectTable::query_stream_at`は、同じ非同期ストレージ境界を通じて保持中の世代を1つ選択します。

最初のポーリングは、`AsyncObjectStore`を通じて選択したコミット済みXBFスナップショットを復旧して読み込み、テーブル全体をDBFへ出力せずに生存行を共有するJSONクエリ契約へ直接対応付けます。

最初の行を返す前にXBFスナップショット全体を読み込んで検証しますが、行の対応付けと配送はポーリングに合わせて行います。

DBFで表現できる値のJSON形式は既存の契約を維持し、XBF固有の値の形式は[XBF](xbf.md)に記載します。

読み込みは`Pending`を返すことがあり、futureのホスト契約を通じて呼び出し側を起こす必要があります。
読み込み完了後の行は、安定したインメモリスナップショットから返します。

アダプターはストア実装のスケジューリング動作をそのまま使います。
`SyncObjectStoreAdapter`はready済みfutureを返しますが、ブロッキングなファイルI/Oを非ブロッキングにはしません。

このアダプターが受け付ける制御は、既存のスナップショットストリームと同じfilter、projection、skip、limitです。

対応しないsort、aggregate、pagination、cursorの制御は、ストレージfutureを作成する前に拒否します。

現在または保持中のスナップショットがない場合、ストレージで失敗した場合、またはXBFスナップショットが不正な場合は、`QueryError`の項目を1つ返してからストリームを終了します。

## 3. ホストの責務

ホスト固有のストリーム実装は、共有契約だけでは決められない動作を所有します。

現在のネイティブスレッドアダプターは、プロセス内クエリのワーカースケジューリング、有界バックプレッシャー、waker通知、破棄時キャンセルを提供します。

- ランタイムがポーリングをスケジュールし、タスクのwakerを渡す。
- 非同期の読み書きで項目を利用できるようになった後、生成側がタスクを起こす。
- 生成側がキューまたはチャネルの上限を適用し、消費側が遅い場合の動作を定義する。
- ストリームを破棄したとき、ホストのリソースを解放またはキャンセルする。
- 転送、ストレージ、タイムアウト、ホスト固有のキャンセル失敗を、ストリームの項目エラー型へ対応付ける。

共有クエリ層は、filter、projection、skip、limitの処理と、項目、ストリーム終了、保留中の処理の区別を所有します。

## 4. 他の契約との関係

同期イテレータと有界NDJSON HTTP契約は変更しません。

`AsyncQueryStream`は、ネイティブ、ワーカー、WASM、WASIのアダプター向けのライブラリ境界です。

ネイティブスレッドアダプターは、その具体的なホスト実装の1つです。

ファイルシステムのチャネルを非ブロッキングに変えたり、resume tokenを追加したり、リモートストレージのプロトコルを定義したりはしません。

Worker互換のWeb Streamsアダプターは、インメモリWASMクエリスナップショットとJavaScriptホスト接続型テーブルの両方に、pullスケジューリング、有界キュー、NDJSON転送チャンク、`AbortSignal`キャンセルを提供します。
`WasmObjectTable.query_stream_json`と`query_stream_json_at`は、ランタイム非依存の`AsyncObjectTable`が現在または選択した保持中のXBFスナップショットを読み込んだ後に解決するPromiseを返します。
Rustの`AsyncObjectStore`契約は、`CancellationToken`を受け取る操作単位の**協調キャンセル**を提供します。
各`AsyncObjectQueryStream`は独立したトークンを持ち、`cancel()`と`cancellation_token()`からキャンセルを要求できます。
ストリームを破棄した場合も、そのトークンをキャンセルします。
標準の読み取り、一覧取得、変更操作は、キャンセルが要求されていない場合に限り、最初のpollでストアメソッドを呼び出します。
読み取りと一覧取得の開始後にキャンセルすると、後続のpollで要求を確認した時点でfutureを破棄します。
基盤のI/Oまで停止するかどうかはホストfutureの実装に依存します。
変更操作は、最初のpollより前にキャンセルすると開始せず、受け付け後の書き込みは中断しません。
書き込み中に`cancel()`を呼んだ場合、ストリームは書き込みの結果が確定してから終了します。
ストリーム自体を破棄した場合は、未完了のfutureも破棄され、書き込みの動作はホストfutureのキャンセル規則に従います。
`SyncObjectStoreAdapter`はブロッキングなファイル操作を中断できません。
生成されたWASM Promiseメソッドは、このRustトークンを公開しません。
Worker専用の`query_stream_json_with_signal`と`query_stream_json_at_with_signal`は、代わりにクエリ単位のシグナルをJavaScriptホスト接続型オブジェクトテーブル経由で対応するFetchリクエストへ渡します。
この方法では、並行する別のクエリへ影響させずに、そのスナップショット読み込みを中断します。
WASIのスケジューリングとリモート再試行方針はホスト固有の責務です。

## 5. Worker Web Streamsアダプター

`createWorkerQueryStream`は、同期またはPromiseを返すWASMクエリストリームを標準の`ReadableStream`でラップします。
非同期ストリーム生成が完了してから行を返し、その後の各`pull`はスナップショットを最大1レコードだけ進め、UTF-8のNDJSONチャンクを1つキューへ追加します。
正の`queueSize`ハイウォーターマークによって、Web Streamsのキューへ需要制御を委譲します。
オブジェクトテーブルのクエリは、XBFスナップショット全体を読み込んでから、各pullで要求された行をJSONへ対応付けます。
リモートページをストリーミングするわけではありません。

アダプターは、readerのキャンセルと`AbortSignal`を現在のWASMクエリストリームの`cancel()`メソッドへ伝えます。
不正なクエリ入力、対応しない制御、ライフサイクルエラーはエラーとして伝播し、空の結果へ変換しません。

詳細な境界、エラー分類、生成ラッパーを使う決定的なfixtureは、[Workerクエリストリームアダプター](worker-query-stream.md)に記載します。

## 一次資料と適用範囲

- [Rustの`Context`](https://doc.rust-lang.org/std/task/struct.Context.html)
- [Rustの`Poll`](https://doc.rust-lang.org/std/task/enum.Poll.html)
- [Rustの`Pin`](https://doc.rust-lang.org/std/pin/index.html)
- [`futures` 0.3.34の`Abortable`](https://docs.rs/futures/0.3.34/futures/future/struct.Abortable.html)
- [`futures` 0.3.34の`AbortHandle`](https://docs.rs/futures/0.3.34/futures/future/struct.AbortHandle.html)

これらの資料は、境界が使うタスクコンテキスト、準備状態、waker契約、pinモデルを定義します。
`futures`の資料は、既定のキャンセルラッパーが使うabort handleの境界を定義します。
ホストfutureを破棄したときに基盤I/Oを停止することや、変更操作を取り消すことまでは保証しません。

txBASEのクエリ意味論を定義したり、特定の非同期ランタイムとの互換性を示したりはしません。
