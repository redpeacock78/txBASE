# Raftコンセンサス設計

状態：カタログ用Raft state machineとOpenRaftの永続ログstoreを実装しましたが、サーバーには接続していません。
サーバーは引き続き固定termの単一authorityを使います。

この文書では、目標とする権威、永続化、適用、運用の契約を定義する。
未実装の機能を実装済みとは記述しない。

## 現在の実装状況

### 実装済み

- OpenRaft `=0.9.25`を固定し、`TypeConfig`、version 1のクライアントコマンド、応答を定義する。
- `RaftCatalogStateMachine`はcommit済みコマンドを適用し、カタログ更新、適用済みRaft位置、クライアントの再試行結果を1つのカタログjournal commitで永続化する。
- 空エントリ、membershipエントリ、拒否したコマンドの適用位置を保存する。これらはカタログtransaction IDを進めない。
- クライアントごとの最新応答を保存し、同一要求の再試行、競合、古いsequence、飛び番を区別する。
- snapshotの生成、転送、インストールで、カタログイメージ、適用位置、membership、再試行状態を一緒に扱う。
- カタログのファイル操作にはTokioのblocking worker poolを使う。
- `RaftLogStore`はnode専用ディレクトリにvote、ログエントリ、commit済み位置、最後にpurgeしたlog IDを永続化する。
- ログjournalは長さ付きのSHA-256検証済みJSON recordを使う。不完全な末尾を復旧し、purge後は新しいgenerationへ圧縮する。
- nodeディレクトリをプロセス間で排他ロックする。ストレージテストにはOpenRaftの`testing::Suite`と再起動後の復旧確認を含める。

### 未実装

- Raft nodeの起動、cluster初期化、peer RPC、認証、TLS。
- membershipを操作するCLI、quorum更新、linearizable read、複数nodeの障害テスト。

コマンドはASCIIのclient IDを128 byteまで受け付けます。
正のsequenceと空でないカタログtagが必要です。
transaction stepは1件から1,000件までで、少なくとも1件の更新を含めます。
シリアライズ後のコマンドは1 MiBまでです。
アプリケーション状態とsnapshot全体は、それぞれ64 MiBまでです。
クライアントの再試行記録は期限なく保持します。
状態が上限に達すると、クライアントの記録を安全に削除する契約が定まるまでstate machineはfail-closedになります。

空のカタログは`RaftCatalogStateMachine::open`で初期化できます。
データがあるカタログには、commit済みカタログsnapshotと明示的な`initialize`が必要です。
transaction IDが0の非空カタログには転送できるMVCC imageがないため、初期化を拒否します。
データがあるのにRaft state sidecarがないカタログは、暗黙に初期化せずfail-closedで開きます。

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
カタログのファイル操作にはTokioのblocking worker poolを使います。
プロトコルruntimeとRPC経路は未実装です。
将来のRPCは、必要な永続書き込みが完了する前に成功を返しません。

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

`RaftLogStore`はvote、ログエントリ、commit済み位置、最後にpurgeしたlog IDを永続化する。
`RaftCatalogStateMachine`はmembership、適用済み位置、カタログ状態、再試行結果を永続化する。
append callbackはjournalをディスクへ同期してから完了する。

journalは長さ付きJSON recordとSHA-256 checksumで構成する。
起動時に不完全な末尾frameを切り詰めるが、checksum不一致や不正なログ連番を含む完全なframeでは起動を拒否する。
purgeでは現在の状態を新しいjournal generationへcheckpointしてから、古いgenerationを削除する。

`RaftLogStore::open`はnode専用ディレクトリを排他ロックするため、同じstoreを複数プロセスから同時に開けません。
ログ形式とロックの契約は実装済みですが、Raft nodeの起動とプロトコル復旧は未実装です。

既存の`FileWal`はRaft storageとしてそのまま使えない。
LSNは0から始まり、公開されている保守操作はログ全体を消去するものです。
競合suffixの切り詰めやsnapshotで置き換えたprefixの破棄には対応しない。
既存の`TXWL`形式を変えずにstorage契約を拡張できる場合に限り、frame形式と復旧処理を再利用する。

Raftの適用エントリには、有界なカタログ更新step、期待するカタログschema tag、requestの事前条件、client ID、単調増加するsequenceを格納する。
実装済みstate machineは、OpenRaftのapply interfaceから渡されたエントリを適用します。
カタログのwrite lock内で、期待するカタログtransaction IDとschema tagを検査します。
成功したカタログ更新、**適用済みRaft位置**、再試行結果を1つのjournal commitで公開します。

古いカタログtag、requestの事前条件違反、不正なtransactionなど、決定的に拒否したコマンドも適用済みRaft位置を進めて応答を記録する。
カタログtransaction IDは進めない。
membershipエントリやプロトコル上のno-opもRaft位置だけを進め、カタログtransactionとして扱わない。

再試行可能な更新にはclientごとの単調増加sequenceを持たせる。
state machineはclientごとの最新sequence、request fingerprint、応答を保存する。
同一要求の再試行には保存した応答を返します。
競合する再試行、古いsequence、飛び番のsequenceは、更新を再適用せずに拒否します。
clientのsequence状態はsnapshotに含め、client IDを再利用しません。

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

Raft snapshotには、カタログMVCC image、最後に適用したRaft log ID、有効なmembership、アプリケーションの重複排除状態を含めます。
現在のadapterはアプリケーション状態を`.txbase.raft-state`に保存し、version付きsnapshotを`.txbase.raft-snapshot`に保存します。
snapshot payloadは`TXRF` version 1 headerを使い、`TXRA` version 1のアプリケーション状態を含みます。
インストール時はカタログイメージと両方のsidecarを一緒に公開します。

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

CIのstate machineテストでは、カタログcommitの原子性、再起動後の再試行、sequence拒否、no-opとmembership、snapshotインストールを検証する。
storage adapterのテストでは、OpenRaftの`testing::Suite`と再起動後の復旧確認を実行する。
CIでは決定的な遅延、メッセージ損失、partition、並べ替え、再起動を設定した複数のRaft nodeも検証する。

完了には、termとvoteの永続復旧、競合ログの置換、quorum喪失、leader交代、応答消失後のclient再試行、適用位置の復旧をテストする。
snapshotのインストールとsuffix保持、learnerの追いつき、joint membership変更、leader交代中のlinearizable readもテストする。

ログ永続化、quorum commit、カタログジャーナル公開、適用済み位置の永続化、client応答の各境界でプロセスを強制終了し、再起動後の状態を検証する。
単一nodeの成功やメモリ上のプロトコルテストだけでは、これらの保証を確認できない。

## 一次資料と適用範囲

- [In Search of an Understandable Consensus Algorithm（Raft）](https://raft.github.io/raft.pdf)は、コンセンサスプロトコルとjoint-consensusによるmembership変更を定義する。
- [OpenRaft 0.9.25の文書](https://docs.rs/openraft/0.9.25/openraft/)は、選択した実装とversion 1.0前のAPI状態を説明する。
- [OpenRaftのfeature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/)は、標準Raft modeと一時的な`storage-v2` APIを説明する。
- [OpenRaft `RaftLogStorage`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftLogStorage.html)は、永続ログstorageの契約を定義する。
- [OpenRaft `RaftStateMachine`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftStateMachine.html)は、適用済み状態、エントリの適用、snapshotの契約を定義する。
- [OpenRaftの導入手順とストレージテストスイート](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/)は、アプリケーション用ストレージとネットワークのadapter、および`testing::Suite`を説明する。
- [OpenRaftの動的membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/)は、learnerの追いつきとvoter変更を定義する。

Raft論文はプロトコルの一次資料です。
OpenRaft文書は選択したlibraryのAPIとadapter要件の一次資料です。
カタログcommand形式、永続storage形式、HTTP動作、移行手順、互換性保証はtxBASE側で定義する。
