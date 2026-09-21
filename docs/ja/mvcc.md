# MVCCと過去スナップショット

txBASEは、DBF更新に対する永続的なテーブル単位のMVCCスナップショットを提供します。

この文書では、現在の可視性契約と、残る複数テーブル境界を定義します。

## 1. 現在の契約

成功した単一テーブルの`save_with_wal`更新には、正のトランザクションIDを割り当てます。

DBF WALはクラッシュ復旧用のジャーナルとして残し、`*.txbase.mvcc`サイドカーはそのトランザクションのcommit済みDBFイメージを保持します。

サイドカーには、そのスナップショットをデコードするために必要なmemoとスキーマのイメージも保持します。

永続的なprepare記録と永続的なcommit記録の両方を持つスナップショットだけが、MVCC APIから可視になります。

CLIはcommit済みバージョンを表示し、指定した1つの過去スナップショットを読み取ります。

```bash
txbase mvcc list path/to/users.dbf
txbase mvcc read path/to/users.dbf 2
```

`mvcc read`は、アクティブなレコードをJSONとして返します。

指定するIDは、commit済みスナップショットを識別しなければなりません。

過去スナップショットは読み取り専用であり、新しい現在状態として保存できません。

## 2. commitと復旧

書き込み処理は、既存のテーブル単位ロックを保持します。

DBF WALを書き込んでsyncし、MVCC prepare記録を書き込んでsyncし、DBFと変更されたサイドカーを置き換え、トランザクション状態を保存し、MVCC commit記録を書き込んでsyncします。

commit記録より前にプロセスが停止した場合、次回の通常DBF読み取りがDBF WALを再生し、MVCC commitを完了します。

MVCC prepareを持たない古いDBF WALは、復旧時に復旧済みスナップショットを記録して更新します。

テーブルロックによって、ローカルの書き込み境界を直列化します。

## 3. 分離境界

現在の実装は、テーブル単位のスナップショット可視性です。

行単位のバージョン保存、述語ロック、serializableな競合検出、CLI呼び出しをまたぐ長寿命トランザクションオブジェクトは提供しません。

カタログのトランザクションIDは複数テーブルcommitの順序を示しますが、カタログは過去の複数テーブルスナップショットをまだ公開しません。

`src/transaction/`の低レベルトランザクションエンジンは、独立したWALトランザクションプリミティブです。

## 4. ロードマップ

次のMVCC境界は、1つのカタログcommitに含まれる全テーブルについて、一貫したバージョンを解決するカタログ全体のスナップショットです。

この作業では、スキーマバージョン、存在しないテーブルバージョン、カタログジャーナル復旧、保持期間、ガベージコレクションの規則を明示します。

分散スナップショット、フォロワー読み取り、serializableな競合検出は、さらに後の作業です。

## 一次資料

- [PostgreSQLのトランザクション分離](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQLの並行性制御](https://www.postgresql.org/docs/current/mvcc.html)
- [SQLiteの分離](https://sqlite.org/isolation.html)
- [SQLiteのWAL](https://sqlite.org/wal.html)
- [Gitのコマンドラインインターフェース規約](https://git-scm.com/docs/gitcli)
