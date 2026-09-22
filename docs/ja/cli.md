# CLIコマンド設計

CLIは、Rust APIの上にある検査と保守の境界です。

パスを開く前に、コマンドの読み取り、書き込み、復旧、サーバーの責務が見えるよう、明示的なサブコマンドを使います。

## コマンドの形

トップレベルの形は次のとおりです。

```text
txbase COMMAND [SUBCOMMAND] ARGUMENT...
```

最初のトークンで1つの責務を選びます。
`mvcc`、`wal`、`xbf`、`index`のように、固有のライフサイクルを持つ機能群には入れ子のサブコマンドを使います。

対象が一意な操作では、パスとJSON値を位置引数にします。
`--encoding`、`--schema`、`--bind`、`--keep`のように解釈を変えるものは名前付きオプションにします。

## 責務のグループ

| グループ | コマンド | 境界 |
| --- | --- | --- |
| 読み取りと検査 | `read`、`schema`、`verify`、`catalog`、`verify-catalog`、`wal inspect`、`mvcc list`、`mvcc read`、`mvcc row`、`mvcc row-at`、`mvcc catalog list`、`mvcc catalog read`、`xbf report`、`index verify` | 読み取り専用の出力です。これらのコマンドは、意図的に更新を公開しません。 |
| 作成と更新 | `init`、`insert`、`pack`、`recall`、`schema apply`、`mvcc gc`、`mvcc catalog gc`、`index build`、`index build-compound`、`index rebuild`、`xbf import`、`xbf export` | コマンド契約に従って、DBFバイト列、サイドカー、永続履歴を書き換えることがあります。 |
| コピーと提供 | `backup`、`restore`、`serve`、`serve-catalog` | 別の文書で定義する境界を通して、データをコピーまたは公開します。 |

`schema apply`を`schema`から分けているのは意図的です。
`schema`は現在のメタデータを検査し、`schema apply`は候補サイドカーを検証してインストールします。

## 設計規則

- 読み取り専用の検査コマンドは、検査だけのためにファイルを作成または切り詰めてはいけません。
- 更新コマンドは、対象を所有するDBF、カタログ、XBFのロックと復旧経路を再利用しなければなりません。
- DBFバイト列を変更せずにサイドカーを変更するコマンドは、ヘルプとトピック文書でそのことを明記しなければなりません。
- 新しいコマンドは、失敗時の動作とフィクスチャに合う最小の責務グループへ置く。
- CLIはリポジトリ固有の有界なサブセットを公開できる。馴染みのあるコマンド名だけで、dBASE、MongoDB、Git、SQLiteとの互換性を主張しない。

コマンドの表面は、[Gitのコマンドラインインターフェース文書](https://git-scm.com/docs/gitcli)にある明示的なサブコマンドとオプションの規約を参考にしています。
ただし、txBASEはGitのコマンド群、リポジトリモデル、オプションの意味をコピーしません。

## 範囲

この文書は、txBASE CLIの表面と設計上の判断を定義します。
データベースの整合性、DBF互換性、XBF復旧、MVCC保持、HTTPの意味は、それぞれのトピック文書に残します。
