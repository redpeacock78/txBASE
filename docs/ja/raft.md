# Raftコンセンサス設計

状態：`serve-catalog`は、初期voter集合を明示する任意のOpenRaftモードを提供します。
このモードではquorum更新、線形化可能な読み取りbarrier、認証付きの専用peer listenerを使います。
peer APIは、準備済みnodeをlearnerとして追加し、ログ複製を開始できます。
joint membershipによるvoter変更、membership状態確認CLI、障害注入テストは未実装です。
`--raft-*`を指定しない`serve-catalog`は、従来の固定termレプリケーションを使います。

この文書では、現在のRaft実装境界と、権威、復旧、運用に残る作業を記録します。

## 現在の実装状況

### 実装済み

- OpenRaft `=0.9.25`を固定し、`TypeConfig`、version 2のクライアントコマンド、応答を定義する。
- `RaftCatalogStateMachine`はcommit済みコマンドを適用し、カタログ更新、適用済みRaft位置、クライアントの再試行結果を1つのカタログjournal commitで永続化する。
- 空エントリ、membershipエントリ、拒否したコマンドの適用位置を保存する。これらはカタログtransaction IDを進めない。
- クライアントごとの最新応答を保存し、同一要求の再試行、競合、古いsequence、飛び番を区別する。
- snapshotの生成、転送、インストールで、カタログイメージ、適用位置、membership、再試行状態を一緒に扱う。
- カタログのファイル操作にはTokioのblocking worker poolを使う。
- `serve-catalog`は、node ID、cluster ID、専用データディレクトリ、peer address、初期membershipを指定してOpenRaft nodeを起動する。membershipの初期化には`--raft-bootstrap`を明示する。
- 専用peer listenerはvote、append、snapshot RPCと認証付きlearner追加要求を処理する。要求を2 MiB、RPC timeoutを10秒に制限し、Bearer認証、cluster ID、node ID、有効なmembership、Raft RPCで全nodeが共有するgenesis catalog fingerprintを検証する。
- peer通信ではnodeごとの証明書と秘密鍵を使うHTTPSを利用できる。Bearer token付きHTTPはloopback peer URLだけで許可する。公開catalog listenerはHTTPのままである。
- `/transaction`と名前付きテーブル更新には`X-Txbase-Client-Id`と正の`X-Txbase-Client-Sequence`が必要である。同じ要求の再試行には記録済み結果を返す。
- 通常のcatalog読み取り前にOpenRaftの線形化可能な読み取りbarrierを呼び出す。明示的にstaleなfollower読み取りは提供しない。
- 3 nodeのCIテストでquorum commit、再試行の重複排除、commit後の認証付きlearner追加、learnerの追いつき、catalogの収束を検査する。
- `RaftLogStore`はnode専用ディレクトリにvote、ログエントリ、commit済み位置、最後にpurgeしたlog IDを永続化する。
- ログjournalは長さ付きのSHA-256検証済みJSON recordを使う。不完全な末尾を復旧し、purge後は新しいgenerationへ圧縮する。
- nodeディレクトリをプロセス間で排他ロックする。ストレージテストにはOpenRaftの`testing::Suite`と再起動後の復旧確認を含める。

### 未実装

- joint membershipによるvoter昇格と削除、およびmembershipや状態を確認するCLI。
- 空のcatalogからの参加。learnerには現在、clusterと同一のcommit済みgenesis imageを事前に用意する必要がある。
- quorum喪失、partition、メッセージ損失や並べ替え、leader交代、再起動、交代中の読み取りを検査する決定的な障害テスト。
- peer HTTPSの証明書とホスト名検証を対象にした統合テスト。
- mutual TLSと公開catalog listenerのTLS。

コマンドはASCIIのclient IDを128 byteまで受け付けます。
正のsequenceと空でないカタログtagが必要です。
transaction stepは1件から1,000件までで、少なくとも1件の更新を含めます。
シリアライズ後のコマンドは1 MiBまでです。
アプリケーション状態とsnapshot全体は、それぞれ64 MiBまでです。
クライアントの再試行記録は期限なく保持します。
状態が上限に達すると、クライアントの記録を安全に削除する契約が定まるまでstate machineはfail-closedになります。

空のカタログは`RaftCatalogStateMachine::open`で初期化できます。
データがあるカタログでは、初期nodeに`--raft-bootstrap`を、準備済みpeerに`--raft-initialize-catalog`を指定し、commit済みcatalog snapshotを用意します。
transaction IDが0の非空カタログには転送できるMVCC imageがないため、初期化を拒否します。
すべての初期voterとlearnerに同じcatalog imageを用意します。
peer RPCはgenesis fingerprintが異なるnodeを拒否します。
learnerを追加する前に、commit済みcatalog snapshotからcatalogを準備し、`--raft-initialize-catalog`でRaft catalog stateを初期化します。
operatorが初期化を明示しない限り、Raft state sidecarがないデータ付きcatalogをfail-closedで開きます。
現在のreaderは`TXRA` state version 2を受け付け、version 1のsidecarを自動移行しません。

## 1. 目的と境界

Raftモードでは、更新を受け付けるカタログ**leader**は同時に1つです。
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

サーバーはOpenRaftのためにTokioのmulti-thread runtimeを作成します。
state machineはカタログのファイル操作を`tokio::task::spawn_blocking`へ渡し、同期peer HTTP clientもTokioのblocking poolで実行します。
書き込み応答は、OpenRaftがcommitし、state machineが永続適用した後に返します。

## 3. ノードIDとmembership

各nodeは、正の固定node ID、client address、peer RPC address、専用の永続データディレクトリを持ちます。
データディレクトリにはcluster IDとnode IDを記録し、catalogディレクトリとは分離します。
1つのnodeデータディレクトリを同時に使えるプロセスは1つです。

clusterの初期化は、初期voter集合を指定する明示的な一度限りの操作です。
すべての初期nodeで同じ`--raft-initial-member ID=URL`集合を指定し、1 nodeだけに`--raft-bootstrap`を指定します。
nodeの再起動時にmembershipを暗黙に初期化または置換しません。

認証済みcluster operatorは、現leaderのpeer listenerへ`POST /raft/v1/learner`を送り、準備済みlearnerを追加できます。
JSON bodyには`version`、`cluster_id`、`node_id`、`peer_address`を含め、`TXBASE_REPLICATION_TOKEN`のBearer credentialを指定します。
応答`202`はOpenRaftがログ複製を開始したことを示し、learnerが追いついたことまでは示しません。
参加nodeの`--raft-initial-member`には、自身のpeer URLと、最初のRPCを送る既存peerを含めます。
これにより、参加nodeはcommit済みmembershipを受け取る前に既存peerを認証できます。
現在のmembershipに同じnode IDとpeer URLがある場合は`200`を返し、競合するIDまたはURLは拒否します。

learnerにはclusterと同じcommit済みgenesis imageを事前に用意し、`--raft-initialize-catalog`で初期化します。
空のcatalogからcluster fingerprintを引き継ぐ機能はありません。
現在のAPIはvoterの昇格や削除を行いません。
新しいvoterを昇格するときは、learnerが追いついた後にOpenRaftのjoint membership手順で変更をcommitします。
ローカル設定の編集だけで投票権を変えることはできません。

起動CLIは、重複node ID、重複peer URL、local nodeのmembership欠落、レプリケーションmodeの混在、異なるnode identity、同じデータディレクトリを使う複数processの起動を拒否します。
voter変更とmembership状態確認の操作は未実装です。

## 4. 永続ログとカタログへの適用

`RaftLogStore`はvote、ログエントリ、commit済み位置、最後にpurgeしたlog IDを永続化する。
`RaftCatalogStateMachine`はmembership、適用済み位置、カタログ状態、再試行結果を永続化する。
append callbackはjournalをディスクへ同期してから完了する。

purge済みのindex以下のentryはappend batchから破棄し、残ったsuffixだけを保存する。
batch内の全entryがpurge済みの場合、`RaftLogStore`はappendを何も保存せず成功させる。
appendが保持中のログに重なる場合、`RaftLogStore`は残ったentryの先頭index以降のsuffixを置き換える。
commit済み位置以下への置換と、ログindexが連続しないappendは拒否する。

journalは長さ付きJSON recordとSHA-256 checksumで構成する。
起動時に不完全な末尾frameを切り詰めるが、checksum不一致や不正なログ連番を含む完全なframeでは起動を拒否する。
purgeでは現在の状態を新しいjournal generationへcheckpointしてから、古いgenerationを削除する。

`RaftLogStore::open`はnode専用ディレクトリを排他ロックするため、同じstoreを複数プロセスから同時に開けません。
起動時は、要求を受け付ける前に永続ログstoreとstate machineを開きます。

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

通常のカタログ読み取りでは、ローカルの適用済み状態を読む前に`Raft::ensure_linearizable()`を呼び出します。
barrierを満たせない場合、serverはstaleな読み取りを返さず`503`を返します。
現在のHTTP modeは読み取りをleaderへ転送せず、staleなfollower読み取りも提供しません。

## 6. peer転送とセキュリティ

peer RPCは、`/raft/v1/vote`、`/raft/v1/append`、`/raft/v1/snapshot`で投票、ログ複製、snapshotインストールを行うversion 1内部envelopeを使います。
認証付き`POST /raft/v1/learner`も同じpeer listenerとBearer tokenを使います。
peer listenerは公開catalog listenerと分離し、各要求を2 MiB、RPC timeoutを10秒に制限します。
Raft RPCの送信nodeについて、有効なmembership、cluster ID、共有genesis fingerprintを照合します。
OpenRaft vote内の送信node IDも検証します。

すべてのRaft peer要求に`TXBASE_REPLICATION_TOKEN`のBearer認証情報が必要です。
loopback以外のpeer URLにはHTTPSを使い、nodeごとに`--raft-peer-cert`と`--raft-peer-key`を指定します。
clientはOSの信頼機構で証明書とホスト名を検証します。
Bearer token付きHTTPはloopback URLだけで使えます。
TLS設定はpeer listenerだけに適用され、公開catalog listenerはHTTPのままです。

## 7. snapshot、復旧、移行

Raft snapshotには、カタログMVCC image、最後に適用したRaft log ID、有効なmembership、アプリケーションの重複排除状態を含めます。
現在のadapterはアプリケーション状態を`.txbase.raft-state`に保存し、version付きsnapshotを`.txbase.raft-snapshot`に保存します。
snapshot payloadは`TXRF` version 1 headerを使い、`TXRA` version 2のアプリケーション状態を含みます。
インストール時はカタログイメージと両方のsidecarを一緒に公開します。

既存の`ReplicationSnapshot`にはカタログのレプリケーション位置がある。
しかし、Raft membershipとcommit済みRaft log IDは含まない。
そのまま完全なRaft snapshotとしてインストールできない。

起動時はnode IDと永続Raft状態を検証する。
snapshotを復元してからcommit済みエントリを順に再生する。
再生完了後に読み書きを提供する。
カタログイメージと適用済み位置が一致しないnodeは、一方を暗黙に選ばず、fail-closedで復旧を要求する。

固定termの`TXRP`authorityからの移行は手動です。
既存writerを停止し、authorityのcatalog imageを1つ選んで検証とbackupを行います。
すべての初期Raft voterを同じimageから準備します。
初期membershipを指定して1 nodeをbootstrapし、他nodeのRaft stateを明示的に初期化します。
peer APIは事前準備済みlearnerを追加できますが、空nodeの参加やcluster snapshotからの初期化には対応しません。
voter集合を推測したり、古いfollowerログを自動で昇格したりしません。

## 8. 検証と完了条件

CIのstate machineテストでは、カタログcommitの原子性、再起動後の再試行、sequence拒否、no-opとmembership、snapshotインストールを検証する。
storage adapterのテストでは、OpenRaftの`testing::Suite`と再起動後の復旧確認を実行する。
3 nodeの統合テストでは、初期voter 2 nodeでのquorum commit、認証付きlearner追加、ログの追いつき、要求再送、全catalogの収束を検査する。
メッセージ遅延、損失、partition、並べ替え、再起動、leader交代の注入は未実装です。

完了には、termとvoteの永続復旧、競合ログの置換、quorum喪失、leader交代、応答消失後のclient再試行、適用位置の復旧をテストする。
snapshotのインストールとsuffix保持、purge後のsnapshotを使ったlearnerの追いつき、joint membership変更、leader交代中のlinearizable readもテストする。

ログ永続化、quorum commit、カタログジャーナル公開、適用済み位置の永続化、client応答の各境界でプロセスを強制終了し、再起動後の状態を検証する。
単一nodeの成功やメモリ上のプロトコルテストだけでは、これらの保証を確認できない。

## 一次資料と適用範囲

- [In Search of an Understandable Consensus Algorithm（Raft）](https://raft.github.io/raft.pdf)は、コンセンサスプロトコルとjoint-consensusによるmembership変更を定義する。
- [OpenRaft 0.9.25の文書](https://docs.rs/openraft/0.9.25/openraft/)は、選択した実装とversion 1.0前のAPI状態を説明する。
- [OpenRaftのfeature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/)は、標準Raft modeと一時的な`storage-v2` APIを説明する。
- [OpenRaft `RaftLogStorage`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftLogStorage.html)は、永続ログstorageの契約を定義する。
- [OpenRaft `RaftStateMachine`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftStateMachine.html)は、適用済み状態、エントリの適用、snapshotの契約を定義する。
- [OpenRaftの導入手順とストレージテストスイート](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/)は、アプリケーション用ストレージとネットワークのadapter、および`testing::Suite`を説明する。
- [OpenRaftのcluster初期化](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/cluster_formation/)は、一度限りの`Raft::initialize()`を定義する。
- [OpenRaftのnetwork trait](https://docs.rs/openraft/0.9.25/openraft/network/)は、peer RPC adapterの契約を定義する。
- [OpenRaftの動的membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/)は、learnerの追いつきとvoter変更を定義する。

Raft論文はプロトコルの一次資料です。
OpenRaft文書は選択したlibraryのAPIとadapter要件の一次資料です。
カタログcommand形式、永続storage形式、HTTP動作、移行手順、互換性保証はtxBASE側で定義する。
