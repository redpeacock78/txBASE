# Raftコンセンサス設計

状態：**Raft**をコンセンサスプロトコルとして選択した。
リポジトリにはRaft runtimeをまだ組み込んでいない。
現在のカタログサーバーは固定termの単一authorityです。

この文書では、目標とする権威、永続化、適用、運用の契約を定義する。
未実装の機能を実装済みとは記述しない。

## 1. 目的と境界

Raft導入後は、更新を受け付けるカタログ**leader**は同時に1つです。
leaderは、現在のmembershipで定まる**quorum**を構成するnodeでログエントリの永続化を確認してから、更新をcommitする。
安定したmembershipでは**voter**（投票権を持つnode）の過半数をquorumとする。
**joint membership**では、旧configと新configの双方で過半数を要求する。

複製する**state machine**は、commit済みログを順に適用して状態を作る処理であり、txBASEではカタログがその状態を持つ。
Raftはleaderの選出と順序付きcommit済みログを提供するが、`OperationIr`、`TransactionStep`、カタログジャーナルを置き換えない。

現在の`ReplicationLog`はRaftログではない。
termを1つに固定し、indexをカタログtransaction IDに結び付け、カタログジャーナルを通じて直接エントリを適用する。
Raftログにはmembership変更やプロトコル上のエントリも入るため、Raftの位置とカタログtransaction IDを分離する。

この権威設計だけでは、カタログをまたぐ分散transaction、partitioning、任意のfollowerからのlinearizable readは提供しない。
これらには別の契約が必要です。

## 2. プロトコルの実装

OngaroとOusterhoutの論文で定義されたRaftを使う。
選挙、ログ照合、quorum commit、membershipの安全性は独自実装せず、OpenRaft `=0.9.25`を統合する。
標準Raft（1 termにつきleaderは1つ）の動作にするため、`single-term-leader`を有効にする。
version付きRPCと保存データのserializationには`serde`を使う。
分離されたログとstate machineのstorage interfaceには`storage-v2`を使う。

OpenRaftはプロトコルエンジン、動的membership操作、アプリケーション用ストレージの適合テストスイートを提供する。
このreleaseの`storage-v2`featureは一時的と明記されている。
storage adapterの移行が済むまで、依存versionとfeature集合を固定する。

選択したOpenRaftのreleaseは、version 1.0より前のAPIが不安定であると明記している。
依存versionとlockfileを固定し、この文書に記載したストレージスイートと複数nodeの障害テストを通さずに更新しない。

プロトコル処理にはOpenRaftのTokio runtimeを使う。
カタログとログのファイル操作でruntimeのexecutor threadを塞がない。
必要な永続書き込みが完了する前にRPCの成功を返さない。

## 3. ノードIDとmembership

各nodeは、正の固定node ID、client address、peer RPC address、専用の永続データディレクトリを持つ。
1つのnodeデータディレクトリを同時に使えるプロセスは1つです。

clusterの初期化は、初期voter集合を指定する明示的な一度限りの操作にする。
nodeの再起動時にmembershipを暗黙に初期化または置換しない。

voterを追加するときは、まず**learner**（投票権を持たないnode）として追加して追いつかせる。
その後、OpenRaftのjoint membership手順でmembership変更をcommitする。
voterの削除や置換も同じcommit済みmembership経路を使う。
ローカル設定の編集だけで投票権を変えない。

運用CLIには、明示的な初期化、参加、membership変更、状態確認の操作が必要です。
重複node ID、異なるcluster ID、同じデータディレクトリを使う複数nodeの起動を拒否する。

## 4. 永続ログとカタログへの適用

OpenRaft用storage adapterは、vote、ログエントリ、membership状態、snapshot、commit済み位置、適用済み位置を永続化する。
appendまたはvoteへの成功応答は、その状態がプロセス再起動後も残ることを意味する。
ログ形式は途中で切れたレコードや破損を検出し、検証できたprefixだけから復旧する。

既存の`FileWal`はRaft storageとしてそのまま使えない。
LSNは0から始まり、公開されている保守操作はログ全体を消去するものです。
競合suffixの切り詰めやsnapshotで置き換えたprefixの破棄には対応しない。
既存の`TXWL`形式を変えずにstorage契約を拡張できる場合に限り、frame形式と復旧処理を再利用する。

Raftの適用エントリには、有界なカタログ更新step、期待するカタログ表現、requestの事前条件、client ID、sequenceを格納する。
state machineはcommit済みエントリだけを適用する。
成功した更新と**適用済みRaft位置**および再試行結果を、カタログジャーナルで原子的に公開する。

事前条件によって拒否したrequestも、適用済みRaft位置を進めて安定した応答を記録する。
ただし、カタログtransaction IDは進めない。
membershipエントリやプロトコル上のno-opもRaft位置だけを進め、カタログtransactionとして扱わない。

再試行可能な更新には、clientごとの単調増加sequenceを持たせる。
clientは同じIDについて同時に1つだけ更新を送る。
state machineはclientごとの最新sequenceと応答を保存する。
最新sequenceと同じ再試行には保存した応答を返し、古いsequenceは再適用せず拒否し、飛び番のsequenceも拒否する。
clientのsequence状態はsnapshotに含め、client IDを再利用しない。

適用済み位置とrequest結果はカタログ更新とともに永続化する。
クラッシュ後の再生はその位置より後から始める。
commit済み更新を二重適用したり、未commitの更新を成功と報告したりしない。

既存の`TXRP`サイドカーは、コンセンサス導入前のレプリケーション形式として維持する。
termが固定され、すべてのentryがカタログtransaction sequenceを進める前提のため、Raft WALとしては使えない。

## 5. clientの更新と読み取り

更新は、Raftがquorumにcommitし、ローカルstate machineが永続的に適用した後に限り成功する。
quorumを失った場合はunavailableまたはnot-leaderを返し、ローカルだけの更新には切り替えない。

通常のカタログ読み取りでは、quorumで確認した**linearizable read**か、ローカルの適用済み状態を読む前のleader read barrierが必要です。
followerが過去時点またはstaleな読み取りを提供する場合は、その位置を応答に含める。
その読み取りをlinearizableとは扱わない。

## 6. peer転送とセキュリティ

peer RPCには、投票、ログ複製、snapshotインストールを扱うversion付き内部プロトコルを使う。
peer listenerは公開カタログlistenerと分離する。
要求サイズとtimeoutに上限を設け、送信nodeを有効なmembershipと照合する。

host境界を越えるpeer通信は認証と暗号化が必要です。
平文のBearer tokenではcredentialとカタログ更新をネットワーク上から読み取れるため、保護として不十分です。
既存のRustls依存は転送層に利用できる。
現在のHTTPレプリケーションルートはRaft RPCとserver側TLSのいずれも提供しない。

## 7. snapshot、復旧、移行

Raft snapshotには、カタログイメージ、最後に適用したRaft log ID、有効なmembership、アプリケーションの重複排除状態を含める。
受信側は、対象範囲のログprefixを削除する前に、これらを一緒に公開する。

既存の`ReplicationSnapshot`にはカタログのレプリケーション位置がある。
しかし、Raft membershipとcommit済みRaft log IDは含まない。
そのまま完全なRaft snapshotとしてインストールできない。

起動時はnode IDと永続Raft状態を検証する。
snapshotを復元してからcommit済みエントリを順に再生する。
再生完了後に読み書きを提供する。
カタログイメージと適用済み位置が一致しないnodeは、一方を暗黙に選ばず、fail-closedで復旧を要求する。

固定termの`TXRP`authorityからの移行は明示的に行う。
既存writerを停止し、authorityのカタログイメージを1つ選んで検証とbackupを行う。
そのイメージからRaft clusterを初期化し、生成したsnapshotを他nodeへ配布する。
voter集合を推測したり、古いfollowerログを自動で昇格したりしない。

## 8. 検証と完了条件

サーバーで使う前に、storage adapterはOpenRaftの`testing::Suite`を通過する。
CIでは、決定的な遅延、メッセージ損失、partition、並べ替え、再起動を設定できる複数のRaft nodeも検証する。

完了には、termとvoteの永続復旧、競合ログの置換、quorum喪失、leader交代、応答消失後のclient再試行、適用位置の復旧をテストする。
snapshotのインストールとsuffix保持、learnerの追いつき、joint membership変更、leader交代中のlinearizable readもテストする。

ログ永続化、quorum commit、カタログジャーナル公開、適用済み位置の永続化、client応答の各境界でプロセスを強制終了し、再起動後の状態を検証する。
単一nodeの成功やメモリ上のプロトコルテストだけでは、これらの保証を確認できない。

## 一次資料と適用範囲

- [In Search of an Understandable Consensus Algorithm（Raft）](https://raft.github.io/raft.pdf)は、コンセンサスプロトコルとjoint-consensusによるmembership変更を定義する。
- [OpenRaft 0.9.25の文書](https://docs.rs/openraft/0.9.25/openraft/)は、選択した実装とversion 1.0前のAPI状態を説明する。
- [OpenRaftのfeature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/)は、標準Raft modeと一時的な`storage-v2` APIを説明する。
- [OpenRaftの導入手順とストレージテストスイート](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/)は、アプリケーション用ストレージとネットワークのadapter、および`testing::Suite`を説明する。
- [OpenRaftの動的membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/)は、learnerの追いつきとvoter変更を定義する。

Raft論文はプロトコルの一次資料です。
OpenRaft文書は選択したlibraryのAPIとadapter要件の一次資料です。
カタログcommand形式、永続storage形式、HTTP動作、移行手順、互換性保証はtxBASE側で定義する。
