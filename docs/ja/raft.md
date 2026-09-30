# Raftコンセンサス設計

状態：`serve-catalog`は、初期voter集合を明示する任意のOpenRaftモードを提供します。
このモードではquorum更新、線形化可能な読み取りbarrier、認証付きの専用peer listenerを使います。
peer APIと`txbase raft membership` CLIは、learner追加、有効なmembershipの照会、joint consensusによるvoter変更を提供します。
空catalogのlearnerは、genesis catalogが空または非空のclusterへ参加できます。
Raft統合テストで、quorum喪失、leader交代、ログの再同期、分断されたnodeの再起動、membership復旧、snapshot追いつきを検査します。
5 nodeのfailoverテストは、同じ分断・復旧シナリオを4台のpeerに保留した非空`AppendEntries`要求の24通りすべての解放順序で実行します。
各シナリオで4台に1件ずつ保留した要求を、replacement leader側のquorum commit後に解放します。
遅延させた古い要求を配送した後も、対象peerのcatalogがtransaction ID 3と`Failover`レコードを維持することを確認します。
分断された旧leaderでは読み取りbarrierが失敗し、新leaderはbarrierが成功した場合だけ選択されることも確認します。
旧leaderへのHTTP `GET /catalog`が`503 raft_unavailable`を返すことも検査します。
replacement leaderの選出後に残るvoter間のRPCを遮断すると、そのleaderへのcatalog読み取りは`503`で失敗します。
通信を復旧すると、read barrierの成功後に`GET /catalog`が`200`を返すことも確認します。
3 nodeの`/transaction`テストは、実HTTP要求をローカルTCP proxy経由で送ります。
proxyはcommit後の最初の成功応答を破棄します。
同じ要求の再試行が同一のJSON結果とtransaction IDを返し、更新が重複適用されず、同じclient sequenceの異なる要求が`409`になることを検査します。
別のmembership復旧テストでは、両方の新voterへの最終uniform構成の`AppendEntries`を保留して旧leaderを停止し、生存voterが同じ変更要求を再送して収束することと、旧leaderがlearnerとして再参加することを検査します。
別の3 nodeテストでは、voter 1台を分断した状態で残るquorumが4件のコマンドをcommitし、leaderでsnapshotを作って対象ログをpurgeします。
接続を戻したvoterがsnapshotをインストールして追いつき、次のclient sequenceを適用することも検査します。
子プロセステストでは、3つの論理nodeを動かすプロセスを4つの永続化境界で強制終了し、同じnodeディレクトリから再起動して同一要求を再試行します。
RAFT-006では、1つのleaderから同一peerへ送る連続した非空`AppendEntries`要求2件も遅延させます。
遅延は次の2つのlog indexを指定して設定し、RPCを解放するまで再試行にも適用します。これにより、membership要求やタイムアウト後の再試行が遅延を迂回しません。
1件目の保留中に2件目の遅延を設定し、1件目を解放してから2件目を解放します。
逐次遅延中はcurrent leaderだけをvoterにし、ほかのnodeをlearnerにします。対象peerにはcurrent leader以外から送信できないようにします。
全nodeが追いついた後に、元のvoter構成へ戻します。
OpenRaft 0.9.25はtargetごとに複製taskを1つ実行し、各`append_entries` futureの完了を待つため、このテストが扱うのは逐次要求です。同一leaderから同じpeerへの同時呼び出しではありません（[task構成](https://github.com/databendlabs/openraft/blob/v0.9.25/openraft/%73rc/docs/internal/threading.md)、[複製実装](https://github.com/databendlabs/openraft/blob/v0.9.25/openraft/%73rc/replication/mod.rs)）。
同じ5 nodeテストの後半では、1台のvoterを隔離したまま、残るquorumで2件のcommandをcommitします。
1件目のcommandがvoterに適用された後で、2件目のlog indexを含む`AppendEntries`要求を遅延させて解放します。
配送後に両方のrecordが1回だけ適用されることを検査します。
固定4要求の解放順序、同一peerへの逐次要求、2件のcommandでvoterをcatch-upさせるシナリオ以外の要求batch、term、partition条件は未検証です。
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
- 専用peer listenerはvote、append、snapshot、learner準備、learner追加、membership状態照会、voter変更の要求を処理する。要求を2 MiB、RPC timeoutを10秒に制限し、Bearer認証、cluster ID、node ID、有効なmembership、Raft RPCで全nodeが共有するgenesis catalog fingerprintを検証する。
- peer通信ではnodeごとの証明書と秘密鍵を使うHTTPSを利用できる。`--raft-peer-client-ca`を指定するとmTLSが有効になり、すべての初期memberでHTTPSが必要になる。nodeはmTLS peerへ接続するとき、peer証明書と鍵をclient identityとして提示する。各peerは設定したCAを使ってidentityを検証する。送信側clientはOSの信頼機構でserver証明書とホスト名を検証する。公開catalog listenerには独立したTLSとmTLSの設定がある。詳細は[公開カタログlistenerの通信保護](catalog-listener-security.md)を参照する。
- `RaftMembershipHttpClient`と`txbase raft membership`は、認証付きの状態照会、learner追加、voter変更を提供する。共有HTTP transportの証明書検証を使い、任意のクライアント証明書と鍵に対応し、version付き応答とvoter集合の整合性を検証する。
- `/transaction`と名前付きテーブル更新では`X-Txbase-Client-Id`と正の`X-Txbase-Client-Sequence`を指定する。同じ要求の再試行には記録済み結果を返す。
- 通常のcatalog読み取り前にOpenRaftの線形化可能な読み取りbarrierを呼び出す。明示的にstaleなfollower読み取りは提供しない。
- 3 nodeのCIテストでquorum commitと再試行の重複排除、昇格前のlearner同期、joint membershipによるvoter昇格と降格、learner停止後に残るvoterでのquorum更新を検査する。
- 2 nodeのCIテストで、genesis catalogが空のclusterへ空catalogのlearnerが参加できることを検査する。
- 3 nodeのCIテストで、空catalogのlearnerに非空genesis catalogとcommit済み更新をsnapshot転送することを検査する。
- 子プロセステストで、3つの論理nodeを動かすプロセスをログ永続化、commit位置の永続化、カタログ更新と適用位置の原子的な公開、OpenRaftの応答受信後に強制終了する。再起動後に同じ要求を再試行し、更新が一度だけ適用されることを検査する。
- 3 nodeのCIテストで、voter 1台を分断したまま残るquorumが4件をcommitし、leaderがsnapshot対象ログをpurgeした後、復旧したvoterがsnapshotから追いついて次のclient sequenceを適用することを検査する。
- 3 nodeの`/transaction`テストは、commit後にローカルTCP proxyで最初の成功HTTP応答を破棄する。同一要求の再試行が同じJSON結果とtransaction IDを返し、更新が一度だけ適用され、同じsequenceの異なる要求が`409`になることを検査する。
- joint membership変更中に旧leaderを停止し、同じ変更要求を生存voterから再送して収束させ、旧leaderをlearnerとして再参加させる3 nodeテストを実行する。
- peer RPCのHTTPSテストで、信頼済みserver証明書を受け入れ、未信頼またはhost不一致のserver証明書を拒否し、mTLSでは信頼済みclient証明書を要求する。client証明書がない接続も拒否する。
- Raft peer listenerの統合テストで、信頼済みclientがmembership routeへ到達できることと、client証明書がない場合や未信頼の場合にTLS negotiationで失敗することを検査する。
- `RaftLogStore`はnode専用ディレクトリにvote、ログエントリ、commit済み位置、最後にpurgeしたlog IDを永続化する。
- ログjournalは長さ付きのSHA-256検証済みJSON recordを使う。不完全な末尾を復旧し、purge後は新しいgenerationへ圧縮する。
- nodeディレクトリをプロセス間で排他ロックする。ストレージテストにはOpenRaftの`testing::Suite`と再起動後の復旧確認を含める。

### 未実装

- 固定4要求の解放順序、同一peerへの逐次2要求、2件のcommandを使うvoter catch-upで2件目の要求を遅延させるケースを超えるスケジュール。異なるtermとpartition条件の組み合わせも未検証である。

コマンドはASCIIのclient IDを128 byteまで受け付けます。
正のsequenceと空でないカタログtagが必要です。
transaction stepは1件から1,000件までで、少なくとも1件の更新を含めます。
シリアライズ後のコマンドは1 MiBまでです。
アプリケーション状態とsnapshot全体は、それぞれ64 MiBまでです。
クライアントの再試行記録は期限なく保持します。
状態が上限に達すると、クライアントの記録を安全に削除する契約が定まるまでstate machineはfail-closedになります。

空のカタログは`RaftCatalogStateMachine::open`で初期化できます。
空catalogのlearnerは、genesis catalogが空または非空のclusterへ参加できます。
非空genesis catalogの場合、leaderは候補nodeにcatalog data、snapshot、適用済みRaft state、membership、client state、永続Raft logがないことを確認します。
候補nodeはcluster fingerprintを採用し、OpenRaftがlearnerとして登録する前にleaderの最新snapshotを受け取ります。
その後、通常のログ複製でsnapshot以降の変更に追いつきます。
既存stateがある候補nodeには、clusterと一致するgenesis fingerprintが必要です。
データがあるカタログでは、初期nodeに`--raft-bootstrap`を、準備済みpeerに`--raft-initialize-catalog`を指定し、commit済みcatalog snapshotを用意します。
transaction IDが0の非空カタログには転送できるMVCC imageがないため、初期化を拒否します。
すべての初期voterに同じcatalog imageを用意します。
learnerは空から開始できますが、異なるgenesis fingerprintの採用は状態を検証したjoin経路に限ります。
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

認証済みcluster operatorは、現leaderのpeer listenerへ`POST /raft/v1/learner`を送り、learnerを追加できます。
JSON bodyには`version`、`cluster_id`、`node_id`、`peer_address`を含め、`TXBASE_REPLICATION_TOKEN`のBearer credentialを指定します。
応答`202`は非同期の参加処理を開始したことを示し、learnerが追いついたことまでは示しません。
leaderは候補nodeの`/raft/v1/learner/prepare`を呼び出し、通常のsnapshot RPCでsnapshotを転送してからOpenRaftへ登録します。
参加nodeの`--raft-initial-member`には、自身のpeer URLと、最初のRPCを送る既存peerを含めます。
これにより、参加nodeはcommit済みmembershipを受け取る前に既存peerを認証できます。
現在のmembershipに同じnode IDとpeer URLがある場合は`200`を返し、競合するIDまたはURLは拒否します。

空catalogのlearnerは、genesis catalogが空または非空のclusterへ参加できます。
非空genesis catalogの場合、catalogとRaft logが空でRaft stateが未初期化であるときに限り、候補nodeはcluster fingerprintを採用できます。
leaderは候補nodeをmembershipへ追加する前にsnapshotをインストールします。
そのため、通常のログ複製はcatalog imageと適用済み位置を引き継いだ後に始まります。
既存stateがある候補nodeには、clusterと一致するfingerprintが必要です。

認証付き`GET /raft/v1/membership`で、nodeごとの有効なmembershipを確認する。
応答にはnode ID、leader ID、server state、有効なmembership log index、voter config、voter IDとlearner ID、nodeのaddressとrole、membership変更の実行中状態を含む。
応答はOpenRaft metricsに基づくnode単位の表示であり、cluster全体のlinearizable readではない。
joint configは複数のvoter集合として返す。

voter集合は次の4段階で変更する。

1. `GET /raft/v1/membership`から、有効なvoter集合とmembership log indexを取得する。
2. 現leaderに認証付き`POST /raft/v1/membership`を送り、`version`、`cluster_id`、`expected_membership_log_index`、`expected_voter_ids`、変更後の`voter_ids`を指定する。
   安定したmembershipから実際に変更するときは、期待するindexとvoter ID集合をcompare-and-swap（CAS）の条件として検証する。
   変更先のvoter集合が既に安定していれば`200`を返し、変更を受け付けた場合は`202`を返す。
3. 昇格するlearnerは、OpenRaftのblocking `add_learner`操作で同期を待ってから`change_membership`へ渡す。
   OpenRaftはjoint configをcommitした後に、変更先の単一configをcommitする。
   txBASEは`retain=true`を指定するため、voter集合から外したnodeはlearnerへ降格し、cluster memberとして残る。
   node metadataは削除せず、learnerへのログ複製も停止しない。
4. `GET /raft/v1/membership`を繰り返し、`effective_voter_configs`が1集合になり、`membership_change_in_progress`が`false`になるまで待つ。

古い要求、未知のnode、followerへの要求、競合するjoint変更には`409`を返す。
leader停止後もjoint configが残ります。
新leaderへ同じ変更先を指定した要求を再送すると処理を再開できます。
ローカル設定の編集だけで投票権は変わらない。

起動CLIは、重複node ID、重複peer URL、local nodeのmembership欠落、レプリケーションmodeの混在、異なるnode identity、同じデータディレクトリを使う複数processの起動を拒否します。
CLIには`raft membership status`、`add-learner`、`change-voters`があります。これらはローカルのカタログディレクトリを開きません。
3つのコマンドすべてに`TXBASE_REPLICATION_TOKEN`が必要です。共有HTTP clientはloopback以外のHTTPでtokenを送らず、HTTPSでは証明書とホスト名を検証します。
状態照会は任意のpeerへ送れますが、そのnodeのローカルmetricsを返します。
learner追加とvoter変更は現在のleaderへ送ります。
voter変更では、直前に取得した状態のmembership log indexとvoter ID集合をcompare-and-swap条件として指定します。

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
認証付き`POST /raft/v1/learner/prepare`、`POST /raft/v1/learner`、`GET`/`POST /raft/v1/membership`は同じpeer listenerとBearer tokenを使います。
準備routeはcommit済みmembershipがない候補nodeに対しても、既知の初期memberからの要求を受け付けます。
fingerprintの変更を許可するのは、候補nodeがpristineかつ未初期化の場合だけです。
learner処理はmembership登録前に、通常のsnapshot routeを使ってchunk単位でsnapshotを転送します。
登録後はOpenRaftが後続log entryを複製します。
peer listenerは公開catalog listenerと分離し、各要求を2 MiB、RPC timeoutを10秒に制限します。
Raft RPCの送信nodeについて、有効なmembership、cluster ID、共有genesis fingerprintを照合します。
OpenRaft vote内の送信node IDも検証します。

すべてのRaft peer要求に`TXBASE_REPLICATION_TOKEN`のBearer認証情報が必要です。
loopback以外のpeer URLにはHTTPSを使い、nodeごとに`--raft-peer-cert`と`--raft-peer-key`を指定します。
clientはOSの信頼機構でserver証明書とホスト名を検証します。
Bearer token付きHTTPはloopback URLだけで使えます。

`--raft-peer-client-ca PEM`を指定すると、そのCAが発行したクライアント証明書をHTTPS peer接続で必須にします。
有効化する場合は、すべての初期member URLにHTTPSを指定します。
各nodeは`--raft-peer-cert`と`--raft-peer-key`を送信時のclient identityにも使います。
そのため、server認証とclient認証の両方に使える証明書を指定します。
client CAは接続してくるpeerのidentityを検証します。
送信側clientは引き続きOSの信頼機構でpeer server証明書を検証します。

Raft peer mTLSは公開catalogのmTLSとは独立しています。
`--raft-peer-client-ca`はpeer listenerだけに適用し、公開catalog clientの証明書検証には[公開カタログlistenerの通信保護](catalog-listener-security.md)で説明する`--tls-client-ca`を使います。
`txbase raft membership`では、`--tls-client-cert`と`--tls-client-key`で別のclient identityを指定できます。

Rustlsの[`WebPkiClientVerifier`](https://docs.rs/rustls/0.23.45/rustls/server/struct.WebPkiClientVerifier.html)は、信頼するrootを設定するとクライアント証明書を必須にして検証します。
[`ConfigBuilder::with_client_auth_cert`](https://docs.rs/rustls/0.23.45/rustls/struct.ConfigBuilder.html#method.with_client_auth_cert)で送信側のidentityを設定します。

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
peer APIはclusterと一致するgenesis identityを持つlearnerを追加でき、非空clusterのsnapshotから空catalogのlearnerも安全に初期化できます。
voter集合を推測したり、古いfollowerログを自動で昇格したりしません。

## 8. 検証と完了条件

CIのstate machineテストでは、カタログcommitの原子性、再起動後の再試行、sequence拒否、no-opとmembership、snapshotインストールを検証する。
storage adapterのテストでは、OpenRaftの`testing::Suite`と再起動後の復旧確認を実行する。
membershipの統合テストでは、quorum commit、空learnerへのsnapshot転送、昇格と降格、降格後に残るvoterでのquorum更新を検査する。
failoverの統合テストは、5 node clusterで同じ分断・復旧シナリオを24通りすべての解放順序で実行します。
4台に1件ずつ保留した非空`AppendEntries`要求を、replacement leader側のquorum commit後に解放します。
各要求の配送後に、対象peerがtransaction 3と`Failover`レコードを保持することも確認する。
各実行では現在のleaderを他の4 nodeから分断し、leaderの更新がカタログへ適用されず、読み取りbarrierが失敗することを確認する。
残るquorumが次のclient sequenceをcommitし、read barrier成功後にreplacement leaderを選出した後で保留要求を解放する。
partitionの復旧と分断nodeの再起動後に、カタログとmembershipが収束することも確認する。
2 nodeのテストでは、genesis catalogが空のclusterへのlearner参加を引き続き検査する。
別の3 nodeテストではvoter 1台を分断し、残るquorumで4件をcommitしてsnapshotを作成します。
leaderがsnapshot対象ログをpurgeした後に接続を戻し、遅延voterのsnapshot適用、追いつき、次のclient sequenceの適用を確認します。
統合テストは、状態照会、learner追加、昇格、冪等な再試行、降格で型付きmembership clientも検査する。CLIテストはコマンド振り分けとvoter IDの入力検証を確認する。
3 nodeの`/transaction`再試行テストでは、ローカルTCP proxyがcommit後の成功HTTP応答を破棄します。
同じ要求の再試行が同一のJSON結果とtransaction IDを返し、更新が一度だけ適用され、同じsequenceの異なる要求が`409`になることを検査します。
failoverテストでは、分断された旧leaderの読み取りbarrierが失敗し、新leaderの選出時にはbarrierが成功することも検査します。
旧leaderへのHTTP `GET /catalog`が`503 raft_unavailable`を返すことも確認します。
replacement leaderの選出後に残るvoter間のRPCを遮断し、そのleaderへのcatalog読み取りが`503`で失敗することを検査します。
通信を復旧し、read barrierの成功後にleaderへの`GET /catalog`が`200`を返すことも確認します。
子プロセステストでは、通常ログエントリを同期した直後、commit markerを同期した直後、カタログ更新と適用位置およびclient再試行結果を1つのjournal commitで公開した直後、OpenRaftが適用結果を返した直後にプロセスを強制終了します。
各境界で親プロセスが3つのnodeディレクトリを再起動し、同じclient IDとsequenceを再試行して、全カタログがtransaction 2に収束し更新が1回だけ適用されることを確認します。
3つの論理nodeは同じ子プロセスで動くため、このテストは単一voterだけを個別に停止する障害を扱いません。
failoverテストでは、current leaderから同一peerへ送る非空`AppendEntries`要求2件を連続して遅延させ、順に解放します。
各遅延はcommandのlog indexを指定し、RPCを解放するまで再試行にも適用します。membership要求やタイムアウト後の再試行は遅延を迂回しません。
別nodeからの複製が追いつき確認を先に満たさないよう、要求を保留している間は対象peerへの送信をcurrent leader以外で遮断します。
全nodeの追いつきを確認してから、元のvoter構成へ戻します。
OpenRaft 0.9.25はtargetごとに複製taskを1つ実行し、各`append_entries` futureの完了を待つため、このテストは同一leaderから同一peerへの逐次要求を扱います。
同じ5 node実行で1台のvoterを隔離したまま、残る4 nodeが2件のcommandをcommitするシナリオも検査します。
1件目のcommandがvoterへ適用された後で、2件目の`AppendEntries`要求を保留します。
全nodeで各recordを1回だけ適用し、transaction 7へ収束します。
固定4要求の解放順序、同一peerへの逐次要求、2件のcommandでvoterをcatch-upさせるシナリオ以外の要求batch、term、partition条件は未検証です。

## 一次資料と適用範囲

- [In Search of an Understandable Consensus Algorithm（Raft）](https://raft.github.io/raft.pdf)は、コンセンサスプロトコルとjoint-consensusによるmembership変更を定義する。
- [OpenRaft 0.9.25の文書](https://docs.rs/openraft/0.9.25/openraft/)は、選択した実装とversion 1.0前のAPI状態を説明する。
- [OpenRaftのfeature flags](https://docs.rs/openraft/0.9.25/openraft/docs/feature_flags/)は、標準Raft modeと一時的な`storage-v2` APIを説明する。
- [OpenRaft `RaftLogStorage`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftLogStorage.html)は、永続ログstorageの契約を定義する。
- [OpenRaft `RaftStateMachine`](https://docs.rs/openraft/0.9.25/openraft/storage/trait.RaftStateMachine.html)は、適用済み状態、エントリの適用、snapshotの契約を定義する。
- [OpenRaftの導入手順とストレージテストスイート](https://docs.rs/openraft/0.9.25/openraft/docs/getting_started/)は、アプリケーション用ストレージとネットワークのadapter、および`testing::Suite`を説明する。
- [OpenRaftのcluster初期化](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/cluster_formation/)は、一度限りの`Raft::initialize()`を定義する。
- [OpenRaftのnetwork trait](https://docs.rs/openraft/0.9.25/openraft/network/)は、peer RPC adapterの契約を定義する。
- [OpenRaftの複製task](https://docs.rs/openraft/0.9.25/openraft/docs/internal/threading/index.html)は、target nodeごとに1つの複製taskを実行することを説明する。
- [OpenRaftの動的membership](https://docs.rs/openraft/0.9.25/openraft/docs/cluster_control/dynamic_membership/)はlearnerの追いつきとvoter変更を定義する。leader交代またはクラッシュがuniform configのcommit前に起きるとjoint configが残ることも、[`Raft::change_membership`](https://docs.rs/openraft/0.9.25/openraft/raft/struct.Raft.html#method.change_membership)に記載されている。
- [OpenRaftのsnapshot複製](https://docs.rs/openraft/0.9.25/openraft/docs/protocol/replication/snapshot_replication/)は、chunk単位のsnapshot転送を説明する。
- [OpenRaft `Raft::install_snapshot`](https://docs.rs/openraft/0.9.25/openraft/raft/struct.Raft.html#method.install_snapshot)は、learner参加で使うsnapshot install RPCを定義する。

Raft論文はプロトコルの一次資料です。
OpenRaft文書は選択したlibraryのAPIとadapter要件の一次資料です。
カタログcommand形式、永続storage形式、HTTP動作、移行手順、互換性保証はtxBASE側で定義する。
