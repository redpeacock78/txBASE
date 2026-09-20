# Firebase のデータモデルと同期から得た知見

Firebase は一つのストレージモデルではありません。

Cloud Firestore と Firebase Realtime Database は異なるトレードオフを選ぶため、txBASE は別々の参照先として扱います。

問うべきなのは Firebase をまねる方法ではありません。

ファイルネイティブなデータベースで、ローカル状態、耐久性、競合、認可の境界をどれだけ明示するかが問題です。

## 1. Cloud Firestore

Firestore は文書をコレクションに保存します。

文書はネストしたオブジェクトを含められ、サブコレクションを持てます。

[Firestore のデータモデル](https://firebase.google.com/docs/firestore/data-model)は、テーブルと行を主な抽象として公開しません。

このモデルでは、文書の識別子とコレクションパスが API の一部になります。

Firestore のトランザクションとバッチ書き込みは目的が異なります。

[トランザクションとバッチ書き込みの文書](https://firebase.google.com/docs/firestore/manage-data/transactions)によると、次のように動作します。

- トランザクションは文書の読み書きをアトミックに行う。
- 競合する編集で再試行が起きると、トランザクション関数が複数回実行されることがある。
- トランザクションでは書き込みの前に読み取りを行わなければならない。
- クライアントがオフラインの場合、トランザクションは失敗する。
- バッチ書き込みは読み取りなしで複数の書き込みをアトミックにコミットする。

再試行規則は大きなアプリケーション契約です。

トランザクションコールバックは複数回実行しても安全でなければなりません。

外部副作用を再試行可能なコールバックの中に隠してはいけません。

Firestore は[競合と直列化可能分離](https://firebase.google.com/docs/firestore/transaction-data-contention)も文書化しています。

保証されるのはコミットされたデータベース状態であり、コールバックが一度だけ実行されることではありません。

クライアントライブラリはトランザクションを再試行することがあり、有限量の競合でもトランザクションが中断することがあります。

Firestore は[書き込み時集約](https://firebase.google.com/docs/firestore/solutions/aggregation)も文書化しています。

これは元の文書が変わるとアプリケーションが集計文書を更新する方法で、読み取りコストを書き込みの複雑さと交換し、集計が古くなった場合の修復方針を必要とします。

## 2. Realtime Database

Realtime Database は参照でアドレスする JSON ツリーを保存します。

[データ保存ガイド](https://firebase.google.com/docs/database/admin/save-data)は、`set`、`update`、`push`、トランザクション操作を区別します。

`set` はパスの値を置き換えます。

`update` は無関係な兄弟を置き換えず、選択した子パスを変更します。

`push` は生成した子キーを作ります。

トランザクションは現在値を読み、変更後の値を計算し、別 writer が先に値を変えた場合に再試行します。

[Web の読み書きガイド](https://firebase.google.com/docs/database/web/read-and-write)は、ローカルイベントと非同期のサーバー同期もクライアントへ見える状態にします。

ツリーモデルと文書モデルは安易に混ぜてはいけません。

DBF テーブルには固定された物理レコードとフィールド記述子があり、任意のネストパスではありません。

## 3. セキュリティと検証規則

Realtime Database のセキュリティ規則は複数の関心を分けます。

| 規則 | 役割 |
| --- | --- |
| `.read` | 読み取りを認可する |
| `.write` | 書き込みを認可する |
| `.validate` | それ以外の認可が済んだ後、書き込み後のデータを検証する |
| `.indexOn` | パスに対してクエリで使うインデックスを宣言する |

[セキュリティ文書](https://firebase.google.com/docs/database/security)は、サーバー側の強制と既定拒否の姿勢を説明します。

Realtime Database の `.indexOn` 規則はクエリパスのインデックス宣言であり、認可の許可ではありません。

Firestore もデータベース操作とセキュリティ規則の評価を分け、アトミック書き込み後のビューを検証する `getAfter()` 検査を含みます。

検証規則はストレージ形式の制約の代わりにはなりません。

認可の判断は誰が操作できるかに答えます。

スキーマ制約は結果の値が有効かに答えます。

インデックス宣言はクエリを効率よく提供する方法に答えます。

txBASE は現在、Firebase の規則言語も認証モデルも実装していません。

将来 HTTP 認可層を追加する場合も、これらの関心は分けて保つべきです。

## 4. オフラインとローカルファーストの動作

Firebase クライアントライブラリは、サーバーが書き込みを受け付ける前にローカル状態を公開できます。

その動作には耐久性のラベルが必要です。

[Realtime Database のオフラインガイド](https://firebase.google.com/docs/database/android/offline-capabilities)は、書き込みのキューを説明し、Realtime Database のトランザクションがアプリ再起動をまたいで永続化されないことを記します。

メモリ上のローカルイベント、キューに入ったクライアント書き込み、同期済み WAL レコード、公開済み DBF 置換は四つの異なる状態です。

これらを同じ種類の成功として呼び出し側へ報告してはいけません。

現在の txBASE のパスからロードした更新契約は、より狭いものです。

1. 更新をロード済み DBF スキーマに対して検証する。
2. 永続的な意図または状態ペイロードを WAL に追記する。
3. WAL を同期する。
4. 現在のファイルプロトコルが許す範囲で DBF と memo サイドカーをアトミックに置き換える。
5. 中断後の起動時にサポート対象の意図または状態ペイロードを再実行する。

現在のサーバーには、クライアントキャッシュ、リスナーストリーム、オフラインキュー、last-write-wins 方針はありません。

## 5. 識別と競合に関する知見

Firebase では、安定したパスまたは文書キーが読み書きの中心になります。

txBASE は現在、HTTP パスに 1 始まりの物理 DBF レコード番号を使います。

この選択は DBF レイアウトを保ちますが、クライアントへ物理識別子を公開します。

現在の更新層は削除済みレコード番号を再利用しません。

将来の論理識別子には、一意性規則、移行動作、インデックスまたは検索契約が必要です。

現在のテーブルロックは保存経路を直列化します。

別にロードされた古いテーブルは自動マージせず拒否します。

これは Firebase の同期ではなく、意図した競合方針です。

## 6. 新しい契約が必要なもの

次の機能はこの文書からは導かれません。

- リアルタイムリスナーまたは変更フィード。
- オフラインクライアント永続化。
- 認証または Firebase セキュリティ規則の評価。
- last-write-wins のマージ意味論。
- 複数文書または複数テーブルのトランザクション。
- サーバーが管理する集計文書。

それぞれを実装するには、同時変更に対する失敗動作、復旧動作、テストが必要です。

## 主な参照先

- [Firestore data model](https://firebase.google.com/docs/firestore/data-model)
- [Firestore transactions and batched writes](https://firebase.google.com/docs/firestore/manage-data/transactions)
- [Firestore transaction contention and serializable isolation](https://firebase.google.com/docs/firestore/transaction-data-contention)
- [Firestore write-time aggregation](https://firebase.google.com/docs/firestore/solutions/aggregation)
- [Realtime Database save data](https://firebase.google.com/docs/database/admin/save-data)
- [Realtime Database web read and write](https://firebase.google.com/docs/database/web/read-and-write)
- [Realtime Database security](https://firebase.google.com/docs/database/security)
- [Realtime Database offline capabilities](https://firebase.google.com/docs/database/android/offline-capabilities)
