# txbase

`txbase`は、dBASE互換のDBFを中心に据えたRust製データベースの初期実装です。

現在の実装は、DBFの読み取り、JSON出力、HTTPの`GET`、RFC 10008に基づく`QUERY`、DBF mutationの実行に範囲を限定しています。

file-backed WALとsnapshot transactionのcoreを実装しています。
DBF-only mutationは`TXDB`、memoを含むmutationは`TXDM` snapshotをWALへ書き込み、syncしてからDBF/sidecarをatomic replaceします。
起動時には未完了のsnapshotを復旧します。
xBase互換フロントエンド、細粒度のWAL record、複数writerの調停は後続工程です。

## 現在できること

- classic DBFの32バイトfield descriptorを読み取る。
- dBASE Level 7の48バイトfield descriptorを判別する。
- header、field descriptor、record、削除フラグを読み取る。
- 代表的な文字列、日付、数値、論理値、整数、倍精度値をJSONへ変換する。
- Visual FoxProのwidth 8の`B` double fieldを読み書きする。
- dBASE Level 7の`+` auto-increment fieldを省略した新規recordへnext valueを割り当て、descriptorを進める。`+` fieldはread-onlyで、明示値のinsertと既存recordの変更を拒否する。
- コマンドラインから有効なrecordをJSONとして出力する。
- `GET /records`と`GET /records/{id}`を提供する。
- `QUERY /records`でfilter、sort、projection、skip、limitを実行する。
- `POST /records`、`PUT /records/{id}`、`PATCH /records/{id}`、`DELETE /records/{id}`を提供する。
- 対応するJSON mutationを`TXDB`または`TXDM` snapshot WALへsyncしてからDBF/sidecarへ保存し、未完了の保存を起動時に復旧する。
- siblingの`.dbt`と`.fpt` sidecarからtext memoを読み、変更時は新しいblockへappendする。
- `B`/`G`のbinary sidecar blockはhex textとして読み、dBASE IV DBTとFPTではhex textの書き込みも新しいbinary blockへのappendとして実行します。dBASE III DBT binary writeは未対応です。
- `PATCH`では通常のfield objectと、型付きの`$set`、`$unset`、`$inc`を使えます。

language-driver IDが`0x01`または`0x02`のcharacter fieldはCP437またはCP850、CP852の代表的なID（`0x1f`、`0x64`）とCP866の代表的なID（`0x26`、`0x65`）、`0x03`または`0x57`はWindows-1252として読み書きします。
それ以外のdriverは既存のUTF-8とlossy fallbackを使います。
選択されたcode pageで表現できない文字の書き込みは拒否します。

## 最短の手順

DBFをJSONとして読み取ります。

```bash
cargo run -- path/to/users.dbf
```

HTTP serverを起動します。

```bash
cargo run -- --serve path/to/users.dbf
```

1-basedのDBF record numberでrecordを取得します。

```bash
curl -s http://127.0.0.1:8080/records/1 | jq
```

有効なrecordを一覧します。

```bash
curl -s http://127.0.0.1:8080/records | jq
```

recordを作成します。

```bash
curl -i -X POST \
  -H 'Content-Type: application/json' \
  -d '{"ID":3,"NAME":"Carol","AGE":42,"ACTIVE":true}' \
  http://127.0.0.1:8080/records
```

recordを部分更新してから論理削除します。

```bash
curl -i -X PATCH \
  -H 'Content-Type: application/json' \
  -d '{"NAME":"Caroline"}' \
  http://127.0.0.1:8080/records/3

curl -i -X DELETE http://127.0.0.1:8080/records/3
```

`QUERY`は`Content-Type: application/json`を要求します。

`QUERY`はactive recordに対してfilter、sort、projection、skip、limitを適用します。

```bash
curl -i -X QUERY \
  -H 'Content-Type: application/json' \
  -d '{"filter":{"AGE":{"$gte":20}},"limit":10}' \
  http://127.0.0.1:8080/records
```

listener addressは`--bind ADDRESS`で変更できます。

## 構成

```text
src/dbf.rs          DBF parserとJSON変換
src/query.rs        JSON query documentとexecutor
src/server.rs       HTTP routing、QUERY境界、DBF mutation
src/storage.rs      range-based storage境界
src/transaction.rs  fileまたはmemory WALとsnapshot transaction
src/xbase.rs        共通operation IR境界
tests/fixtures/     parser test用fixture
docs/research.md    仕様調査と設計判断
```

workspaceは、責務の所有者やbuild上の理由が生じるまで単一packageで保ちます。

## Query実行

query documentはMongoDBのpredicateから必要な表現だけを借りています。

```json
{
  "filter": {
    "AGE": {"$gte": 20, "$lt": 30},
    "COUNTRY": {"$in": ["JP", "TW"]}
  },
  "sort": {"AGE": 1},
  "projection": {"NAME": 1, "AGE": 1},
  "limit": 100,
  "skip": 0
}
```

初期operatorは`$eq`、`$ne`、`$gt`、`$gte`、`$lt`、`$lte`、`$in`、`$nin`、`$and`、`$or`、`$not`です。

missing fieldでは`$ne`と`$nin`が一致し、array valueでは要素のいずれかが条件を満たすと一致します。
sortの同値recordはDBF record orderを保ちます。
Dotted pathとindexは未対応です。

## Mutationの意味

`POST`は新しい物理recordを作成し、`201 Created`と1-basedのrecord locationを返します。
`PUT`は全fieldを置き換え、`PATCH`は通常のfield objectに加えて、`$set`、`$unset`、`$inc`を使うupdate documentも受け付けます。
operatorと通常fieldの混在、および同一fieldへの複数operator適用は拒否します。
`POST`と`PUT`で省略したfieldはDBF nullになりますが、dBASE Level 7の`+` fieldを`POST`で省略した場合はdescriptorのnext valueを割り当てて進めます。`+` fieldはread-onlyで、insert時の明示値と既存recordへの変更を拒否します。
未知のfieldは拒否します。

`DELETE`はDBFの削除markerを設定して`204 No Content`を返します。
削除済みrecord numberは再利用せず、以後の読み取りは`404 Not Found`になります。
serverはDBF-only変更では`TXDB`、memo field変更ではDBFとsidecarを含む`TXDM` snapshotをWALへ書き込み、syncしてから対象ファイルを置き換えます。
`DbfTable::from_path`は中断されたmutationの最新snapshotを復旧します。
WALは細粒度のmutation recordではなく全体snapshotを保存し、複数writerの調停は未対応です。
text memoは`.dbt`または`.fpt`から読み取り、変更時は新しいblockをappendしてDBF pointerも更新します。
sidecarとDBFは`TXDM` WAL snapshotで復旧可能な単位として保存します。
`B`/`G`のdBASE IV DBTとFPT binary block書き込みはhex textとして対応し、dBASE IV DBTのsidecar headerにあるblock sizeも尊重します。dBASE III DBT binary write、OLE semantics、この範囲外のmemo形式は未対応です。

## 検証

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
```

仕様調査の詳細は[docs/research.md](docs/research.md)に記録しています。

英語の概要は[README.md](README.md)です。
