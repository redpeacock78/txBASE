# WASI クエリストリーム

固定したCIコマンドでコンポーネントをビルドし、ディレクトリを事前公開してWasmtimeで実行します。

## 1. ビルドと実行

CIはRustの`wasm32-wasip2`ターゲット向けに`wasi-query-stream` exampleをビルドします。
exampleは`wasip3`を使い、WASI 0.3の`wasi:cli/command`コンポーネントを公開します。
このターゲット固有のexampleはRust 1.87以降を必要とします。
その他のターゲットに対するcrateの最低バージョンは1.85です。

```bash
cargo build --locked --example wasi-query-stream --target wasm32-wasip2 --release
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  /data/users.dbf '{"projection":{"NAME":1}}'
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  --object-store /data/object-store users '{"projection":{"NAME":1}}'
wasmtime run --dir ./data::/data target/wasm32-wasip2/release/examples/wasi_query_stream.wasm \
  --object-store /data/object-store users '{"projection":{"NAME":1}}' --generation 0
```

コマンドは1行に1つのJSONオブジェクトを出力します。
Wasmtimeはコンポーネントのファイル名を第0引数として渡し、その後にDBFのパスとクエリJSONを渡します。
`--dir`はホストのディレクトリへのアクセスを許可し、コンポーネント内では`/data`として公開します。
object-store形式では既定で現在世代を読み込みます。
`--generation`を指定すると保持中の世代を選択します。

## 2. クエリの契約

DBF形式では、コンポーネントは事前公開されたDBFファイル全体をメモリへ読み込み、`DbfTable::from_bytes`、`query::parse`、`query::stream_query`を再利用します。
object-store形式では、既存の`AsyncObjectTable::query_stream`または`query_stream_at`契約を通じて、`namespace/manifest.json`、XBFスナップショット、namespaceのWALディレクトリを読み込みます。
object-storeのrootには、`--dir`で公開したディレクトリを指定します。
既存のオブジェクトテーブル配置では、スナップショットを`namespace/snapshots/`に、復旧記録を`namespace/wal/`に置きます。
ファイルシステムアダプターは読み取り専用であり、`SyncObjectStoreAdapter`を介して同期`std::fs`操作を使います。
このアダプターは操作を非同期traitへ接続しますが、ファイルシステムI/Oをノンブロッキングにはしません。
クエリの実行中にobject storeが変更されないようにしてください。
このアダプターは同時書き込みを調整しません。
`AsyncObjectTable`は読み取り前に保留中のWAL記録を復旧するため、マニフェストの公開やWAL記録の削除が必要な場合、読み取り専用ストアでは失敗します。
スナップショットの読み込みと復旧は、最初の行を出力する前に完了します。

どちらの形式も共有クエリパーサーとクエリストリーム契約を再利用します。
共有するストリーミング制御のうち、`filter`、`projection`、`skip`、`limit`を受け付けます。
行を出力する前に、`sort`、集約、ページネーション、カーソルの制御を拒否します。

コンポーネントは既存の`AsyncQueryStream`をポーリングし、結果を1件ずつUTF-8のNDJSON行に変換します。
行をWASI Component Modelのバイトストリームへ書き込み、`wasi:cli/stdout.write-via-stream`を並行して待機します。
stdout側の消費速度が行の生成へバックプレッシャーとして伝わります。

## 3. エラーとキャンセル

引数、ファイル、DBF、クエリ、stdoutのエラーはstderrへ出力し、コマンドを失敗として終了します。
後続の行でエラーが起きると、それ以前の行がすでに出力されていることがあります。
ストレージ、スナップショット、復旧のエラーは行の生成前に発生します。
CIスモーク検査は、保留中のWAL記録を復旧できず、行を出力しないことを確認します。
拒否されるストリーミング制御も行の出力前に失敗することを確認します。

Rustのクエリストリームを破棄すると、行の生成を終了します。
このコマンドは同期DBF読み込みとobject-storeのファイルシステム読み込みを中断可能にはせず、ホストのタイムアウトやプロセスキャンセルの方針も定義しません。

## 4. CIの範囲

`wasi-query-stream` CIジョブはWASIターゲットとWasmtime `49.0.0`を導入し、コンポーネントをビルドして`tests/wasi_query_stream_smoke.sh`を実行します。
スモーク検査は固定DBF・XBFフィクスチャをデコードし、DBFの結果、現在および保持中のXBF世代を確認します。
`sort`がstdoutを出力せず拒否されることと、保留中のWAL復旧がstdoutを出力せず失敗することも確認します。

この検査が保証するのは、固定したWasmtimeランタイムでのコンポーネントのビルドとCLI動作です。
別のWASIホストへのデプロイ、書き込み可能またはプロバイダー接続型のストレージ、ノンブロッキングなファイルシステムI/Oは保証しません。

## 一次資料と対象範囲

- [WASI 0.3とネイティブ非同期処理](https://wasi.dev/releases/wasi-p3)
- [`wasip3` 0.9.0のバインディング](https://docs.rs/wasip3/0.9.0%2Bwasi-0.3.0/wasip3/)
- [Rustの`wasm32-wasip2`ターゲット](https://doc.rust-lang.org/rustc/platform-support/wasm32-wasip2.html)
- [Wasmtime CLIオプション](https://docs.wasmtime.dev/cli-options.html)
- [Bytecode AllianceのWasmtimeセットアップアクション](https://github.com/bytecodealliance/actions)

WASIの現行リリースは0.3.1であり、WASI 0.3.0で導入された非同期プリミティブに加えてComponent Modelの機能を導入しています。
このアダプターが使うのは0.3.0の`stream`と`future`の境界であり、0.3.1だけが持つWIT機能には依存しません。
