# CLIコマンドリファレンス

CLIは、Rust APIの上にある検査と保守の境界です。

コマンド体系の設計判断は[CLIコマンド体系の設計](cli-design.md)に記載します。

パスを開く前に、コマンドの読み取り、書き込み、復旧、サーバーの責務が見えるよう、明示的なサブコマンドを使います。

## コマンドの形

トップレベルの形は次のとおりです。

```text
txbase COMMAND [SUBCOMMAND] ARGUMENT...
```

最初のトークンで1つの責務を選びます。
`mvcc`、`wal`、`xbf`、`index`のように、固有のライフサイクルを持つ機能群には入れ子のサブコマンドを使います。

対象が一意な操作では、パスとJSON値を位置引数にします。
`--encoding`、`--schema`、`--bind`、`--keep`、`--keep-rows`のように解釈を変えるものは名前付きオプションにします。

## 呼び出しの契約

コマンドを指定しない場合、`-h`、`--help`を指定した場合は、ルートの使用方法を表示して成功します。

未知のトップレベルコマンドまたはオプションはエラーになります。

各コマンドは、繰り返し引数または明示的に受け付けるオプションを除き、必須引数の欠落と余分な末尾引数を拒否します。

現在のCLIは、グローバルな`--`終端、バージョンコマンド、設定ファイル、コマンド単位のhelpモードを持ちません。

パーサーは、選択したコマンドに対して文書化したオプションだけを受け付けます。

## コマンド一覧

次の表が現在のコマンド契約です。

| コマンド | 現在の動作と書き込み境界 |
| --- | --- |
| `txbase read FILE [--encoding NAME]` | DBFのアクティブレコードをJSONで表示する。パスのロード時に、読み取り前に保留中のWALまたはスキーマエクスポートを復旧することがある。 |
| `txbase init FILE --field NAME:TYPE:LENGTH[:DECIMALS]...` | 繰り返し指定したフィールド仕様からclassic DBFを作成し、既存DBFの上書きを拒否する。 |
| `txbase insert FILE JSON_OBJECT` | 1つのJSONオブジェクトを通常のWAL付きテーブル永続化経路で追加する。 |
| `txbase schema FILE [--encoding NAME]` | ロードしたDBFを検証し、スキーマとレコードメタデータをJSONで表示する。 |
| `txbase schema apply FILE SCHEMA_JSON` | 現在のDBFとアクティブレコードに対してスキーマ候補を検証し、スキーマサイドカーだけを置き換える。 |
| `txbase verify FILE [--encoding NAME]` | DBFと、存在する場合はインデックスサイドカーを検証する。シリアライズしたDBFを再解析し、レコード境界も検査する。 |
| `txbase catalog DIRECTORY` | 検出したカタログのスキーマをJSONで表示する。 |
| `txbase verify-catalog DIRECTORY` | 検出したカタログを検証し、成功時に`{"valid":true}`を表示する。 |
| `txbase cdc FILE [--after TRANSACTION_ID]` | 単一テーブルのcommit済みCDCイベントを表示する。`--after`は排他的なトランザクションIDカーソルであり、利用者の確認応答やカーソル状態の保存は行わない。 |
| `txbase cdc catalog DIRECTORY [--after TRANSACTION_ID]` | 同じ排他的カーソル規則で、複数テーブルの原子的なカタログCDCイベントを表示する。 |
| `txbase mvcc list FILE` | commit済みテーブルスナップショットのIDを表示する。 |
| `txbase mvcc read FILE TRANSACTION_ID` | commit済みの過去のテーブルスナップショットを1つ読み取る。 |
| `txbase mvcc row FILE RECORD` | 正の物理レコード番号1つについて、保持中のバージョンを表示する。 |
| `txbase mvcc row-at FILE TRANSACTION_ID EPOCH RECORD` | commit済みトランザクション、行epoch、物理レコード番号で、保持中の行バージョンを1つ読み取る。 |
| `txbase mvcc gc FILE --keep COUNT [--keep-rows COUNT]` | 新しい完全イメージを保持し、任意で物理行ごとの古いバージョンを保持する。MVCC履歴サイドカーだけを置き換える。 |
| `txbase mvcc catalog list DIRECTORY` | commit済みカタログスナップショットのIDを表示する。 |
| `txbase mvcc catalog read DIRECTORY TRANSACTION_ID` | 過去のカタログスナップショットを1つ読み取り、テーブルとレコードを返す。 |
| `txbase mvcc catalog gc DIRECTORY --keep COUNT` | 新しいカタログスナップショットを保持し、カタログMVCC履歴サイドカーだけを置き換える。 |
| `txbase wal inspect WAL` | WALを作成も切り詰めもせずに読み取り、完全なレコードと不完全な末尾を表示する。 |
| `txbase index build FILE FIELD...` | 指定したフィールド群のスカラーインデックスサイドカーを作成して永続化する。 |
| `txbase index build-compound FILE NAME FIELD[:DIRECTION]...` | 名前付き複合インデックスを1つ作成する。方向には昇順の`1`または`asc`、降順の`-1`または`desc`を指定する。 |
| `txbase index verify FILE` | インデックスサイドカーを検証し、そのスキーマをJSONで表示する。 |
| `txbase index rebuild FILE` | 現在のテーブルからインデックスサイドカーを再構築して永続化する。 |
| `txbase xbf import DBF XBF [--encoding NAME]` | DBFを有界なXBFスナップショットへ変換する。 |
| `txbase xbf export XBF DBF [--schema]` | 表現可能なXBFテーブルを出力する。`--schema`は復旧可能なエクスポート境界を通して、表現可能なスキーマメタデータを保つ。 |
| `txbase xbf report XBF` | DBFまたはスキーマサイドカーを書き込まずに、DBFとしての表現可能性を報告する。 |
| `txbase pack FILE [--encoding NAME]` | 論理削除レコードを取り除き、参照中のmemoブロックを圧縮し、既存インデックスを更新し、関連スナップショットをWALで永続化する。 |
| `txbase recall FILE RECORD [--encoding NAME]` | 論理削除されたレコードを1つ、通常の永続化境界で復元する。 |
| `txbase backup SOURCE DEST` | DBFと対応するmemo、スキーマ、CDC、状態、MVCC、有効なインデックスサイドカーを検証してコピーする。 |
| `txbase restore SOURCE DEST` | バックアップをソースとして、同じ検証済みコピー手順を使う。 |
| `txbase serve FILE [--bind ADDRESS] [--encoding NAME]` | 単一テーブルHTTPサーバーを起動する。 |
| `txbase serve-catalog DIRECTORY [--bind ADDRESS] [--replication-term TERM] [--replication-role authority|follower]` | カタログHTTPサーバーと有界なレプリケーション配送およびフォロワー適用位置確認ルートを起動する。既定の`authority`ロールは`/transaction`と名前付きテーブルの更新ルートをカタログジャーナルと`TXRP`サイドカーへ捕捉し、`follower`ロールは直接のカタログ更新とフォロワー適用位置確認を`409`で拒否しながらレプリケーション配送を受け付ける。`TERM`は正の固定ローカルtermで、既定値は`1`。`TXBASE_REPLICATION_TOKEN`を設定した場合、すべてのレプリケーションルートにRFC 6750の`Authorization: Bearer <token>`ヘッダーが必要になる。 |

## オプションの所有範囲

- `--field`は`init`だけが所有し、繰り返し指定できる。
- `--encoding`はDBFテキストをデコードするパスロードコマンドの`read`、`schema`、`verify`、`xbf import`、`pack`、`recall`、`serve`に属する。
- `--schema`は`xbf export`だけに属する。
- `--bind`は`serve`と`serve-catalog`だけに属する。
- `--replication-term`は`serve-catalog`だけに属し、正の固定ローカルtermを選択する。
- `--replication-role`は`serve-catalog`だけに属し、既定の`authority`は書き込みロール、`follower`は直接のカタログ更新と適用位置確認を拒否してレプリケーション配送を受け付ける。
- `TXBASE_REPLICATION_TOKEN`は`serve-catalog`の任意の環境変数であり、CLIオプションではない。コマンドラインにトークンを露出させずにレプリケーションルートを保護する。
- `--after`は`cdc`と`cdc catalog`だけに属する。
- `--keep`はテーブルとカタログのMVCCガベージコレクションに属し、`--keep-rows`はテーブルMVCCガベージコレクションだけに属する。
- `index build-compound`は、各フィールドの方向に`1`または`asc`、`-1`または`desc`を受け付ける。

コマンドが正の値として指定するトランザクションID、epoch、レコード番号、保持件数には、正の値を指定しなければなりません。

## 読み取り、復旧、更新の境界

読み取り指向のコマンドは利用者データの更新を意図的に公開しませんが、ライブテーブルのロードでは通常の復旧境界を実行します。

その境界は、テーブルを読む前に保留中のテーブルWALまたはスキーマエクスポートジャーナルを処理することがあります。

明示的な例外は`wal inspect`です。

このコマンドは完全なレコードと切断された末尾だけを報告します。

WALの作成や切り詰めは行いません。

更新コマンドは、DBF、カタログ、XBFの操作が所有するロックと復旧経路を再利用します。

`schema apply`は検証後にスキーマサイドカーだけを変更します。

`mvcc gc`は検証後に関係する履歴サイドカーだけを変更します。

`index build`、`index rebuild`、`xbf export --schema`は、文書化した対象ファイルに加えて派生またはメタデータのサイドカーを書き込みます。

`backup`と`restore`は同期済み一時ファイルを通して宛先ファイルを1つずつ置き換えるため、中断したコピーは新しい複数ファイルトランザクションプロトコルになりません。

中断後の宛先を使う前に、`txbase verify DEST`を実行します。

単一テーブルサーバーとカタログサーバーは、一度だけ検査するコマンドではなく、長時間動作するプロセスです。

HTTP契約は、[HTTPメソッドの意味](http-semantics.md)、[クエリモデル](query-model.md)、[複数テーブルカタログ](catalog.md)、レプリケーション配送については[分散化の進化](distributed-evolution.md)で定義します。

## 責務のグループ

| グループ | コマンド | 境界 |
| --- | --- | --- |
| 読み取りと検査 | `read`、`cdc`、`cdc catalog`、`schema`、`verify`、`catalog`、`verify-catalog`、`wal inspect`、`mvcc list`、`mvcc read`、`mvcc row`、`mvcc row-at`、`mvcc catalog list`、`mvcc catalog read`、`xbf report`、`index verify` | 読み取り指向の出力です。上記の通常復旧に関する注意を伴います。 |
| 作成と更新 | `init`、`insert`、`pack`、`recall`、`schema apply`、`mvcc gc`、`mvcc catalog gc`、`index build`、`index build-compound`、`index rebuild`、`xbf import`、`xbf export` | コマンド契約に従って、DBFバイト列、サイドカー、永続履歴を書き換えることがあります。 |
| コピーと提供 | `backup`、`restore`、`serve`、`serve-catalog` | 別の文書で定義する境界を通して、データをコピーまたは公開します。 |

`schema apply`を`schema`から分けているのは意図的です。
`schema`は現在のメタデータを検査し、`schema apply`は候補サイドカーを検証してインストールします。

`cdc`は読み取り専用のイベント検査コマンドであり、更新や利用者の確認応答を行うコマンドではありません。

任意の`--after`引数は、テーブルのCDCサイドカーにあるコミット済みイベントをトランザクションIDで排他的に絞り込むカーソルです。

`cdc catalog DIRECTORY`は、複数テーブルのカタログトランザクションがカタログCDCサイドカーへ公開した原子的なイベントを読み取ります。

カタログ形式でも、カタログトランザクションIDに対して同じ排他的な`--after`カーソルを使います。

`mvcc gc FILE --keep COUNT`は、新しい完全イメージのテーブルスナップショットを保持します。
`--keep-rows COUNT`を追加すると、最も古い保持対象の完全スナップショットより前について、物理行ごとに古いバージョンを最大`COUNT`件保持します。
行履歴は同じMVCCサイドカーに保存され、GCは現在のDBFと完全スナップショットを変更しません。

`pack`は削除済みの物理レコードを取り除き、残存レコードを振り直し、参照されるDBT/FPT memoブロックを圧縮し、既存のインデックスサイドカーを更新します。
DBF、memo、インデックス、MVCC、トランザクション状態、CDCの変更は、1つのWAL付き境界で永続化します。
`recall`はスキーマ検証後に削除済みレコード1件を復元し、通常の永続化境界を使います。

## 範囲

この文書は、txBASE CLIの表面とコマンド契約を定義します。
データベースの整合性、DBF互換性、XBF復旧、MVCC保持、HTTPの意味は、それぞれのトピック文書に残します。
