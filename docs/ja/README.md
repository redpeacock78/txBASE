# txBASE ドキュメント

変更する契約に対応する文書から読み始めてください。

現在の動作、調査結果、将来の作業を明示的に区別しています。

## ストレージと互換性

- [DBF と dBASE の互換性](dbf-compatibility.md)
- [変更データ取得](change-data-capture.md)
- [スキーマメタデータとローカル制約](schema-metadata.md)
- [セカンダリインデックスのサイドカー](indexes.md)
- [複数テーブルカタログ](catalog.md)
- [XBF v1 フォーマット草案](xbf.md)
- [MVCCと過去スナップショット](mvcc.md)
- [エッジストレージとオブジェクトストレージのコミット](edge-storage.md)

## クエリとプロトコルの契約

- [クエリモデル](query-model.md)
- [非同期クエリストリーム](async-streaming.md)
- [集約モデル](aggregation.md)
- [結合モデル](joins.md)
- [クエリ計画と外部語彙](query-planning.md)
- [更新モデル](mutation-model.md)
- [スナップショットトランザクション](transactions.md)
- [HTTP メソッドの意味と QUERY](http-semantics.md)

## コマンドラインインターフェース

- [CLIコマンドリファレンス](cli.md)
- [CLIコマンド体系の設計](cli-design.md)

## 品質と調査

- [SQLite のテストと品質モデル](testing-quality.md)
- [品質契約マトリクス](quality-matrix.md)
- [Firebase のデータモデルと同期から得た知見](firebase-model.md)
- [仕様調査インデックス](research.md)
- [ロードマップと明示的な非目標](roadmap.md)

## 将来のアーキテクチャ

- [WASMとワーカーのホスト境界](wasm.md)
- [Worker Fetchオブジェクトストレージアダプター](worker-object-store.md)
- [Workerクエリストリームアダプター](worker-query-stream.md)
- [分散化の進化](distributed-evolution.md)

## ファイル粒度の規則

所有者、失敗時の動作、フィクスチャ、変更頻度のいずれかが異なる場合は文書を分割します。

分割によってナビゲーションだけが増える場合は、関連する契約を1つの文書に保ちます。
