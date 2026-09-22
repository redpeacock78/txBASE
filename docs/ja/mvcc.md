# MVCCと過去スナップショット

txBASEは、DBF更新とカタログトランザクションに対する永続的なMVCCスナップショットを提供します。

この文書では、現在の可視性と保持の契約を定義します。

## 1. 現在の契約

成功した単一テーブルの`save_with_wal`更新には、正のトランザクションIDを割り当てます。

DBF WALはクラッシュ復旧用のジャーナルとして残し、`*.txbase.mvcc`サイドカーはそのトランザクションのcommit済みDBFイメージを保持します。

サイドカーには、そのスナップショットをデコードするために必要なmemoとスキーマのイメージも保持します。

永続的なprepare記録と永続的なcommit記録の両方を持つスナップショットだけが、MVCC APIから可視になります。

CLIはcommit済みバージョンを表示し、指定した1つの過去スナップショットを読み取ります。

```bash
txbase mvcc list path/to/users.dbf
txbase mvcc read path/to/users.dbf 2
txbase mvcc row path/to/users.dbf 1
txbase mvcc row-at path/to/users.dbf 2 1 1
txbase mvcc gc path/to/users.dbf --keep 5 --keep-rows 10
```

`mvcc read`は、アクティブなレコードをJSONとして返します。

`mvcc row`は、物理DBFレコード番号1つについて、保持中のバージョンを表示します。

`mvcc row-at`は、トランザクションID、epoch、物理レコード番号で、保持中の行バージョン1つを読み取ります。

`--keep-rows`を付けると、`mvcc row`は完全イメージのスナップショットが保持対象外になった行バージョンも返せます。
`mvcc row-at`は、保持中の完全スナップショットのトランザクション、または行履歴記録が保持されたトランザクションを受け付けます。

どちらのコマンドもJSONを返し、Rust APIと同じ保持済み履歴の境界を使います。

`mvcc read`で指定するIDは、commit済みの完全スナップショットを識別しなければなりません。

過去スナップショットは読み取り専用であり、新しい現在状態として保存できません。

`mvcc gc`は、commit済みスナップショットのうち新しいものを`--keep`で指定した正の件数だけ保持し、MVCC履歴サイドカーだけを書き直します。

任意の`--keep-rows COUNT`は、最も古い保持対象の完全スナップショットより前について、物理行ごとに古いバージョンを最大`COUNT`件保持します。

GCはテーブルロックを取得してからテーブルWALを復旧し、履歴を同期済み一時ファイルへ書き出して置き換えます。

現在のDBF、memo、スキーマ、index、トランザクション状態の各ファイルは変更しません。

完全イメージ履歴から削除したIDは`mvcc read`で読み取れません。
保持した行履歴のIDは`mvcc row-at`で読み取れ、次のcommitは保持されたIDの後ろに追加されます。

カタログトランザクションは、検出したすべてのテーブルのcommit単位のイメージを保持します。

```rust
use txbase::catalog::Catalog;

let versions = Catalog::mvcc_versions("database")?;
let catalog = Catalog::from_path_at("database", 2)?;
let users = catalog.open_table("users")?;
```

カタログCLIも同じ境界を公開します。

```bash
txbase mvcc catalog list path/to/database
txbase mvcc catalog read path/to/database 2
txbase mvcc catalog gc path/to/database --keep 5
```

1つの過去カタログから開くすべてのテーブルは同じカタログcommit IDを持ち、読み取り専用です。

カタログHTTPサーバーは、そのcommit IDを`?at=<transaction ID>`として、カタログスキーマ、名前付きテーブルのレコード、クエリ、ストリーム、説明、結合の読み取りに受け付けます。

1つのリクエストは、1つのカタログイメージだけを読み取ります。

過去のHTTPクエリと結合は現在のインデックスサイドカーを再利用せず、`at`を付けたカタログ更新は拒否します。

Rustの`Catalog::begin_read` APIは、カタログとテーブルの読み取りロックを解放する前に、検出したすべての現在テーブルを1つの読み取り専用メモリ内イメージへ取得します。

`CatalogReadTransaction`は取得したカタログcommit IDを保持し、後のファイルシステム変更を読み取らずにテーブルを読み取り、有界結合できます。

これはプロセス内の読み取りビューであり、永続的な保持ピンではありません。

保持されたカタログcommitを再び開くAPIは引き続き`Catalog::from_path_at`です。

### 行単位の履歴

テーブルMVCCサイドカーは、通常のcommitの行変更を同じprepareとcommitの記録内に保存します。

公開APIは、保持中の行履歴と、commit済みテーブルスナップショットにおける1行の読み取りを提供します。

```rust
use txbase::dbf::{DbfTable, RowId};

let history = DbfTable::mvcc_row_versions("users.dbf", 1)?;
let row = DbfTable::mvcc_read_row(
    "users.dbf",
    2,
    RowId {
        epoch: history[0].id.epoch,
        record_number: 1,
    },
)?;
```

行番号は物理DBFレコード番号であり、利用者が定義した主キーではありません。

`PACK`、スキーマまたはレコードレイアウトの変更、明示的なレイアウトリセットの後は、epochで行IDを分離します。

論理削除は`deleted: true`の行バージョンとして保存するため、過去の読み取りで、削除された行と保持履歴に存在しなかった行を区別できます。

通常のcommitの行変更は、それを含む完全イメージMVCCレコードにprepareとcommitの両方が存在するときだけ可視になります。

`--keep-rows`を使うと、GCは選択した古い変更を同じサイドカー内の圧縮した行履歴記録へコピーします。
これらの記録は、読み取り可能な完全テーブルスナップショットを新しく作りません。

MVCC GCは、最初に保持するスナップショットを行履歴の基準に再構築し、その後の差分を再計算します。
`--keep-rows`を使うと、物理行ごとに選択した古いバージョンも保持するため、完全イメージの保持期間を超えて行を読み取れます。

カタログMVCCは、カタログcommitごとに完全なテーブルイメージを保存し続けます。

したがって、カタログの過去読み取りは既存のcommit単位契約を維持し、カタログスナップショットに対するテーブル単位の行履歴を公開するとは主張しません。

公開[`DbfTransaction`](transactions.md) APIは、パスで指定した1つのテーブルに対して、オプティミスティックな非公開スナップショットを提供します。

非公開コピーに対して操作とクエリを実行し、1つのWAL付きcommitとして公開するか、コピーを破棄します。

`DbfTransaction::begin_serializable`は、1つのテーブルに対する任意選択の粗粒度serializable境界です。
beginからcommitまたはrollbackまで排他テーブルロックを保持するため、同じロックを使うローカルtxBASEの読み書きはトランザクションと交互に実行されません。

## 2. commitと復旧

書き込み処理は、既存のテーブル単位ロックを保持します。

DBF WALを書き込んでsyncし、MVCC prepare記録を書き込んでsyncし、DBFと変更されたサイドカーを置き換え、トランザクション状態を保存し、MVCC commit記録を書き込んでsyncします。

commit記録より前にプロセスが停止した場合、次回の通常DBF読み取りがDBF WALを再生し、MVCC commitを完了します。

MVCC prepareを持たない古いDBF WALは、復旧時に復旧済みスナップショットを記録して更新します。

テーブルロックによって、ローカルの書き込み境界を直列化します。

カタログトランザクションのjournalは、カタログMVCC履歴ファイルを準備済みまたはコミット済みの変更集合に含めます。

準備済みジャーナルは現在のテーブルイメージと新しい履歴イメージの両方をロールバックします。

コミット済みジャーナルは、次のカタログ読み取りが返る前に両方を再適用します。

## 3. 分離境界

現在の実装は、単一テーブルcommitに対してテーブル単位のスナップショット可視性を提供し、カタログトランザクションに対してcommit単位のスナップショット可視性を提供します。

カタログ履歴は、カタログcommitごとに検出したすべてのテーブルの完全なイメージを保存します。

テーブル単位の行履歴は、完全イメージMVCC GCによる基準保持と、`--keep-rows`による物理行単位の独立した保持を提供します。

公開するテーブル単位トランザクションは既定でオプティミスティックな古い元データの拒否を提供します。
明示的な`DbfTransaction::commit_with_row_merge` APIは、スキーマ、レイアウト、レコード件数が変わらない場合に、テーブルロック内で異なる物理行への更新または削除をマージできます。
insertと同一行への異なる変更は引き続きエラーであり、結果が同じ行は追加の変更がないものとして扱います。
`DbfTransaction::begin_serializable`は、テーブル全体のロックによって1つのテーブルの実行を直列化します。
既定のテーブル単位トランザクションAPIは、述語ロックおよび述語単位のserializable競合検出を提供せず、いずれのAPIもCLI呼び出しをまたぐ長寿命トランザクションオブジェクトを提供しません。

カタログトランザクションIDは、一貫した複数テーブルイメージを識別し、カタログHTTPの1リクエストでそのイメージを選べます。

同じIDは`CatalogReadTransaction`の取得時点を識別するためにも使います。

HTTPの選択値は、長寿命トランザクションオブジェクトを作りません。

Rustの読み取りトランザクションは取得後に安定しますが、述語ロックまたはserializableな競合検出は提供しません。

`Catalog::gc_mvcc`とカタログGCコマンドは、カタログイメージのうち新しいものを`keep_last`で指定した正の件数だけ保持します。

カタログGCはカタログ書き込みロックを取得し、準備済みジャーナルを先に復旧してから、保持対象の履歴を同期済み一時ファイルへ書き出して`.txbase.catalog.mvcc`を置き換えます。

DBF、memo、スキーマ、index、カタログトランザクション状態の各ファイルは変更しません。

`src/transaction/`の低レベルトランザクションエンジンは、独立したWALトランザクションプリミティブです。

## 4. ロードマップ

現在の保持境界は、完全イメージによるテーブルとカタログのスナップショットに対する件数ベースのGCです。

現在の行単位境界は、epochでIDを分離した物理レコード履歴と、件数ベースの圧縮です。

スキーマ移行履歴、述語単位のロック、テーブル横断のserializable検証、長寿命の分散トランザクションは将来の作業です。

分散スナップショット、フォロワー読み取り、serializableな競合検出は、さらに後の作業です。

## 一次資料

- [PostgreSQLのトランザクション分離](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQLの並行性制御](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLiteの分離](https://sqlite.org/isolation.html)
- [SQLiteのWAL](https://sqlite.org/wal.html)
