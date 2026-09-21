# MVCCと過去スナップショット

txBASEは、DBF更新とカタログトランザクションに対する永続的なMVCCスナップショットを提供します。

この文書では、現在の可視性契約と、残る行単位の境界を定義します。

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
txbase mvcc gc path/to/users.dbf --keep 5
```

`mvcc read`は、アクティブなレコードをJSONとして返します。

`mvcc row`は、物理DBFレコード番号1つについて、保持中のバージョンを表示します。

`mvcc row-at`は、トランザクションID、epoch、物理レコード番号で、保持中の行バージョン1つを読み取ります。

どちらのコマンドもJSONを返し、Rust APIと同じcommit済み履歴およびGCの境界を使います。

指定するIDは、commit済みスナップショットを識別しなければなりません。

過去スナップショットは読み取り専用であり、新しい現在状態として保存できません。

`mvcc gc`は、commit済みスナップショットのうち新しいものを`--keep`で指定した正の件数だけ保持し、MVCC履歴サイドカーだけを書き直します。

GCはテーブルロックを取得してからテーブルWALを復旧し、履歴を同期済み一時ファイルへ書き出して置き換えます。

現在のDBF、memo、スキーマ、index、トランザクション状態の各ファイルは変更しません。

GCで削除したIDは読み取れず、次のcommitは保持されたIDの後ろに追加されます。

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

### 行単位の履歴

テーブルMVCCサイドカーは、同じprepareとcommitの記録内に行の変更も保存します。

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

行の変更は、それを含む完全イメージMVCCレコードにprepareとcommitの両方が存在するときだけcommit済みになります。

MVCC GCは、最初に保持するスナップショットを行履歴の基準に再構築し、その後の差分を再計算するため、削除したトランザクションに依存せず保持中の行を読み取れます。

カタログMVCCは、カタログcommitごとに完全なテーブルイメージを保存し続けます。

したがって、カタログの過去読み取りは既存のcommit単位契約を維持し、カタログスナップショットに対するテーブル単位の行履歴を公開するとは主張しません。

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

テーブル単位の行履歴は、既存の完全イメージMVCC GCを通じた件数ベースの保持を提供します。

述語ロック、serializableな競合検出、CLI呼び出しをまたぐ長寿命トランザクションオブジェクトは提供しません。

カタログトランザクションIDは、一貫した複数テーブルイメージを識別します。

`Catalog::gc_mvcc`とカタログGCコマンドは、カタログイメージのうち新しいものを`keep_last`で指定した正の件数だけ保持します。

カタログGCはカタログ書き込みロックを取得し、準備済みジャーナルを先に復旧してから、保持対象の履歴を同期済み一時ファイルへ書き出して`.txbase.catalog.mvcc`を置き換えます。

DBF、memo、スキーマ、index、カタログトランザクション状態の各ファイルは変更しません。

`src/transaction/`の低レベルトランザクションエンジンは、独立したWALトランザクションプリミティブです。

## 4. ロードマップ

現在の保持境界は、完全イメージによるテーブルとカタログのスナップショットに対する件数ベースのGCです。

現在の行単位境界は、epochでIDを分離した物理レコード履歴と、件数ベースの圧縮です。

独立した行保持方針、スキーマ移行履歴、述語ロック、長寿命スナップショットトランザクションは将来の作業です。

分散スナップショット、フォロワー読み取り、serializableな競合検出は、さらに後の作業です。

## 一次資料

- [PostgreSQLのトランザクション分離](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQLの並行性制御](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLiteの分離](https://sqlite.org/isolation.html)
- [SQLiteのWAL](https://sqlite.org/wal.html)
- [Gitのコマンドラインインターフェース規約](https://git-scm.com/docs/gitcli)
