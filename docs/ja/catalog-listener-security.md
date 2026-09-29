# 公開カタログlistenerの通信保護

状態：`serve-catalog`は、任意のサーバーTLSと相互TLS（mTLS）を提供します。

この契約は固定termレプリケーションモードとRaftモードの公開listenerに適用します。
独立したRaft peer listenerの設定は対象外です。

## TLSの設定

`--tls-cert PEM`と`--tls-key PEM`を同時に指定すると、カタログをHTTPSで提供します。
どちらも指定しない場合、`serve-catalog`は従来どおりHTTPを使います。
サーバーは起動時に証明書チェーンと秘密鍵を読み込み、対応を検証してから、ALPNでHTTP/1.1を通知します。
TLSにはRustlsの既定設定を使い、TLS 1.2とTLS 1.3を有効にします。

サーバー証明書と鍵に加えて`--tls-client-ca PEM`を指定すると、指定した信頼CA証明書のいずれかが発行したクライアント証明書を必須にします。
証明書を提示しないクライアントと、信頼できるチェーンを提示できないクライアントは接続できません。
`--tls-client-ca`を省略した場合、クライアント証明書は要求しません。
信頼CAから検証できた証明書には、同じカタログAPIアクセスを認めます。
証明書の識別情報に応じた権限設定と失効リストの検査は行いません。

TLSファイルは起動時に一度だけ読み込みます。
証明書や信頼CAを変更した後は、サーバーを再起動してください。
不正なTLSファイルとlistenerのbindエラーは、カタログを開く前に報告します。

公開listenerの`--tls-cert`、`--tls-key`、`--tls-client-ca`は、Raft peer用の`--raft-peer-cert`、`--raft-peer-key`、`--raft-peer-client-ca`とは独立しています。
Raft用のオプションはpeer listenerだけを設定します。詳細は[Raftコンセンサス設計](raft.md)を参照してください。

## HTTP転送の境界

カタログのroute handlerは引き続き`tiny_http`で処理します。
[`tiny_http`の`SslConfig`](https://docs.rs/tiny_http/0.12.0/tiny_http/struct.SslConfig.html)は証明書と秘密鍵を受け取りますが、クライアント証明書verifierを公開しません。
そのためRustlsのfrontendで公開TLSを終端し、ephemeral portでloopbackにbindした`tiny_http` backendへHTTP/1.1を転送します。
クライアントCAを指定したときは、Rustlsの[`WebPkiClientVerifier`](https://docs.rs/rustls/0.23.45/rustls/server/struct.WebPkiClientVerifier.html)がクライアント証明書を必須とし、チェーンを検証します。

frontendは要求本文と応答本文をバッファにすべて読み込まず、ストリームとして転送します。
転送前に`Connection`が指名するフィールドと既知のhop-by-hopフィールドを取り除き、要求と応答に`Via`を追加します。
これはHTTP intermediaryに求める[RFC 9110の7.6.1節](https://www.rfc-editor.org/rfc/rfc9110.html#section-7.6.1)と[7.6.3節](https://www.rfc-editor.org/rfc/rfc9110.html#section-7.6.3)に従います。
実装では、TLSに[`tokio-rustls`](https://docs.rs/tokio-rustls/0.26.6/tokio_rustls/server/struct.TlsAcceptor.html)を使います。
HTTP転送にはHyperの[HTTP/1 server connection](https://docs.rs/hyper/1.11.1/hyper/server/conn/http1/struct.Builder.html)、[ストリーム本文](https://docs.rs/hyper/1.11.1/hyper/body/index.html)、および[接続を再利用するHTTP client](https://docs.rs/hyper-util/0.1.21/hyper_util/client/legacy/struct.Client.html)を使います。

backendは、プロセス内で生成した256 bitのtokenを専用request headerに含む要求だけを受け付けます。
frontendはクライアントが送った同名headerを破棄してから、内部tokenを付けます。
tokenを持たずloopback backendへ直接送った要求には`403 Forbidden`を返します。
これにより、ローカルHTTP経由で公開listenerのクライアント証明書検査を迂回できません。

## クライアント認証とアプリケーション認証

TLSは通信を保護し、クライアントにサーバーを認証します。
mTLSはさらに、設定したCA証明書までつながるクライアント証明書を認証します。
証明書ごとに異なるカタログ権限を割り当てる機能ではありません。

`TXBASE_REPLICATION_TOKEN`はレプリケーションrouteに使う独立したBearer認証情報です。
HTTPではネットワーク上で読み取られるため、TLSの代わりにはなりません。
組み込みの`replicate catch-up` clientは、`--tls-client-cert`と`--tls-client-key`を両方指定するとクライアント証明書を提示します。
client identityを指定した場合はHTTPSが必要です。server証明書はOSの信頼機構で検証します。

Raft peer TLSと公開カタログTLSでは、listenerと信頼設定がそれぞれ独立しています。
Raft peerのmTLSは`--raft-peer-client-ca`で個別に有効にします。詳細は[Raftコンセンサス設計](raft.md)を参照してください。

## CLIの例

サーバー認証を使うHTTPSを有効にする例です。

```sh
txbase serve-catalog ./data --bind 0.0.0.0:8443 \
  --tls-cert ./server-chain.pem --tls-key ./server-key.pem
```

クライアント証明書を必須にするHTTPSを有効にする例です。

```sh
txbase serve-catalog ./data --bind 0.0.0.0:8443 \
  --tls-cert ./server-chain.pem --tls-key ./server-key.pem \
  --tls-client-ca ./client-ca.pem
```

組み込みcatch-up clientは、次のコマンドでクライアント証明書を提示できます。

```sh
txbase replicate catch-up ./follower https://authority.example \
  --replication-term 4 --follower-id follower-1 \
  --tls-client-cert ./follower-chain.pem --tls-client-key ./follower-key.pem
```

`serve-catalog`に`--raft-*`を指定した場合も、同じ公開TLSオプションを使えます。
秘密鍵はサーバー実行アカウントだけが読めるようにし、このカタログへの接続を認めるclient専用のCAを使ってください。
