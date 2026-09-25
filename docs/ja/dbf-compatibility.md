# DBF と dBASE の互換性

この文書は、公開されているファイル形式とtxBASEが現在実装するサブセットを分けて記述します。

まず、読み取り可能で復旧可能なDBFアクセスを成立させます。

完全なdBASEまたはVisual FoxPro互換性を主張するものではありません。

## 1. 物理 DBF 構造

[dBASE Level 7 ファイル形式](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)は、ヘッダー、フィールド記述子、レコード、任意のファイル終端マーカーという構造を定義します。

重要なヘッダー位置は次のとおりです。

| オフセット | サイズ | 意味 |
| ---: | ---: | --- |
| `0` | 1 | ファイルバージョンと memo 関連フラグ |
| `1..3` | 3 | 年、月、日のバイトによる最終更新日 |
| `4..7` | 4 | リトルエンディアンのレコード数 |
| `8..9` | 2 | リトルエンディアンのヘッダー長 |
| `10..11` | 2 | リトルエンディアンのレコード長 |
| `28` | 1 | 生成時 MDX フラグ |
| `29` | 1 | 言語ドライバー識別子 |
| `32..` | 可変 | フィールド記述子 |
| 記述子末尾 | 1 | フィールド記述子終端 |

従来の記述子は32バイトです。

dBASE Level 7の記述子は48バイトで、記述子終端の後に拡張プロパティを持てます。

記述子は、フィールド名、型、バイトオフセット、幅、小数桁、型固有フラグを含みます。

Level 7の自動インクリメント記述子は、初期値と増分情報も含みます。

レコード領域は宣言されたヘッダー長から始まります。

各物理レコードは削除フラグバイトから始まるため、DBFのレコード幅にはそのバイトが含まれます。

宣言されたレコード数とレコード長は提案ではなく境界です。

読み手は、宣言された構造を越えて読み取る代わりに、切り詰められたヘッダー、記述子、レコードを拒否しなければなりません。

## 2. memo とバイナリのサイドカー

memoフィールドはDBFレコードにブロックポインターを保存し、ペイロードを兄弟memoファイルに保存します。

dBASE形式はDBTサイドカーを、Visual FoxProは一般にFPTサイドカーを使います。

txBASEがサポートする形式では、ブロック0がサイドカーヘッダーです。

ポインターのバイト順とブロックヘッダーのバイト順は形式ごとに異なるため、実装はすべてのmemoファイルを1つの汎用バイト列として扱いません。

現在の書き込み経路は次のとおりです。

| サイドカー | 現在の txBASE の動作 |
| --- | --- |
| dBASE III DBT | 形式の終端規則でテキストとバイナリブロックを追記し、`0x1a1a`バイナリ終端を予約する |
| dBASE IV DBT | サイドカーヘッダーが宣言するブロックサイズでテキストとバイナリブロックを追記する |
| Visual FoxPro FPT | FPT ブロックヘッダーを付けてテキストブロックと型 0 のバイナリブロックを追記する |

[Visual FoxPro のテーブルファイル構造](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm)と[memo ファイル構造](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm)はFoxPro固有の構造を文書化しています。

## 3. txBASE のフィールド対応

パーサーは宣言されたフィールド記述子を読み取り、サポートする値をJSONへ対応付けます。

次の表は一般的なDBF型ガイドではなく、互換性の境界です。

| フィールドの分類 | 現在の動作 |
| --- | --- |
| 文字とテキスト | サポートする言語ドライバー対応でデコードし、表現できない文字の書き込みを拒否する |
| 日付 | 有効なら JSON の日付文字列へ変換する |
| 数値と論理 | 有効なら JSON の数値または真偽値へ変換する |
| 整数と double | 固定幅表現で読み書きする |
| Visual FoxPro `B`、幅 8 | double として扱う |
| Visual FoxPro `Y` | `f64`の丸めを避けるため、四桁小数の固定小数点の文字列として公開する |
| Visual FoxPro `T` | ユリウス日とミリ秒の組から、秒精度の ISO-8601 文字列として公開する |
| Visual FoxPro `V`と`Q` | 可変長テキストまたはバイナリ値は固定レコードスロットと`_NullFlags`規則を使い、`Q`は小文字の 16 進数にする |
| Visual FoxPro `W` | FPT バイナリブロックを小文字の 16 進数として公開する |
| `M` | DBT または FPT のテキスト、またはバイナリフラグ時の小文字の 16 進数 |
| `B`、`G`、`P` | バイナリサイドカーのペイロードを小文字の 16 進数として公開し、Visual FoxPro の`P`は画像ブロックとして扱う |
| Level 7 `+`と FoxPro `0x31` | insert で省略された値を記述子から割り当て、既存値は読み取り専用とする |

Visual FoxProのnullableテーブルは内部で`_NullFlags`を使います。

このフィールドはJSON文書から隠し、影響を受けるinsertまたは更新に対してだけ再生成します。

[Visual FoxPro の可変長フィールドの説明](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm)を`V`と`Q`のスロット規則の参照にします。

## 4. エンコーディングと CJK の境界

言語ドライバーのバイトは、文字バイトをどう解釈するかを宣言します。

txBASEは`src/dbf/codepages.rs`、`src/dbf/codec.rs`、`src/dbf/codec_cjk.rs`が実装するコードページを現在サポートします。

これには、サポートするドライバー IDが使うCP437、CP850、CP852、CP866、Windows-1250、Windows-1251、Windows-1252、Windows-1253、Windows-1254、Windows-1255、Windows-1256の対応が含まれます。

現在のCJKスライスは、サポートするCJKフィクスチャで必要なDBF言語ドライバー IDもデコードおよびエンコードします。

Visual FoxProのIDについては、[Visual FoxPro がサポートするコードページ](https://www.vfphelp.com/help/html/a3d7b0e0-8320-44b1-8983-17c30a78c6c4.htm)を参照してください。

| ドライバー ID | 宣言されたプラットフォーム | 実効コーデック |
| --- | --- | --- |
| `0x7b` | 日本語 Windows | `encoding_rs::SHIFT_JIS`による Windows-31J/CP932 |
| `0x13` | 従来型の日本語 Shift-JIS | `encoding_rs::SHIFT_JIS`による Windows-31J/CP932 |
| `0x4d` | dBASE 簡体字中国語 | GBK/CP936 |
| `0x7a` | 簡体字中国語 Windows | GBK/CP936 |
| `0x4f` | 従来型の繁体字中国語 | Big5/CP950 |
| `0x79` | 韓国語 Windows | EUC-KR/CP949 |
| `0x4e` | 従来型の韓国語 ANSI/OEM | EUC-KR/CP949 |
| `0x78` | 繁体字中国語 Windows | Big5/CP950 |

従来型のID `0x13`、`0x4d`、`0x4e`、`0x4f`には、固定したRuby [`dbf`互換性表](https://github.com/infused/dbf/blob/6b6547384439fd009815d20112b22c58eee83503/README.md#encodings-code-pages)のコードページ対応を使います。

`schema`出力の`encoding`メンバーは、上記4つのCJKコーデックを含む、サポートするすべての宣言済みコードページを識別します。

`encoding_metadata`メンバーは`declared`、`effective`、`source`の値を公開し、読み手が同じ解釈を再現できるようにします。

`source`は`language-driver`、`explicit-override`、`fallback`のいずれかです。

サイドカーと呼び出し時のオーバーライドは、意図的に同じ`explicit-override`値を使います。

コーデックは壊れたバイト列を読み取り時にU+FFFDとして扱います。

DBFバイト列を変更する前に、書き込みで表現できない文字を拒否します。

既存の固定フィールド幅検査はバイト幅検査のままです。

そのため、マルチバイト値が収まらなければ切り詰めず拒否します。

任意の`*.txschema.json`サイドカーの`encoding`プロパティは、4つの宣言済みコーデックまたは明示専用の`Shift_JIS`、`EUC-JP`、`GB18030`、`ISO-2022-JP`コーデックを選べます。

利用できるラベルは`windows-31j`、`cp932`、`shift_jis`、`shift-jis`、`sjis`、`gbk`、`cp936`、`euc-kr`、`cp949`、`big5`、`cp950`、`euc-jp`、`gb18030`、`iso-2022-jp`、`iso2022-jp`です。

正規化した選択値は`schema`出力の`encoding_override`に表示されます。

`encoding_metadata`の`effective`メンバーにも表示されます。

このオーバーライドは文字のデコードとエンコードの前に適用し、DBFヘッダーの言語ドライバーバイトは変更しません。

パス対応のread、schema、verify、pack、recall、serverコマンドは、同じ8つの明示コーデックに対する`--encoding NAME`も受け付けます。

呼び出し時のオーバーライドはサイドカーのオーバーライドより優先し、DBFヘッダーまたはメタデータサイドカーには書き込まず、スキーマ出力の実効`encoding_override`として見えるままです。

明示的な`Shift_JIS`オーバーライドはstrictです。

ASCII、半角カタカナ、JIS X 0208文字を受け付け、CP932拡張バイトは読み取り時にU+FFFDへ変換し、CP932専用文字は書き込みで拒否します。

DBF言語ドライバーバイトは変更しません。

未知のドライバーは既存のUTF-8またはlossyフォールバック動作を保ちます。

選択したコードページで表現できない文字は書き込みで拒否します。

DBFフィールド幅はバイト幅です。

そのため、将来のCJK互換性層は次の項目を一緒に定義しなければなりません。

1. 宣言ドライバーと明示的オーバーライド。
2. デコードとエンコードに使うコーデック。
3. 切り詰めと検証のバイト幅規則。
4. クエリとソートに使う照合規則。
5. 往復動作を証明するフィクスチャ。

Shift_JISとCP932は交換可能なラベルではありません。

同じ注意がEUC-JP、GBK、GB18030、Big5、韓国語エンコーディングにも当てはまります。

互換性テストは、宣言されたWindows-1251ドライバーを持つ上流Visual FoxProの`cp1251.dbf`フィクスチャも往復します。

このテストは、キリル文字のフィールド値を読み取り、書き換えを永続化します。

現在のオーバーライドスライスは、4つの宣言ドライバーコーデックとstrict Shift_JIS、EUC-JP、GB18030、ISO-2022-JPについて、呼び出し時とサイドカーでの選択をカバーします。

固定DBFフィクスチャは4つのVisual FoxPro CJK driver IDとlegacy dBASEの`0x4d` driver IDをカバーし、マルチバイトのレコード値を往復します。

固定した明示コーデックのバイトフィクスチャは、サポートする8つの明示コーデック名についてDBFレコードのデコードと書き込みの往復をカバーします。

クエリソートには、有界なUnicode小文字化モードとUnicode NFKC小文字化モードに加え、版付きのICU4X 2.1.1日本語、中国語、韓国語ロケール照合があります。

これらのソート規則はDBFコードページから独立しており、ロケール全体の互換性を主張しません。

より広い上流CJK照合フィクスチャは今後の作業です。

上流のJavaDBF GBKフィクスチャは、GBKでエンコードされた3つのCJKフィールド名と28件の実データをカバーします。
読み取り、更新、バイト列の往復も検証します。

上流のRust `dbase-rs` CP936フィクスチャも、実際のGBKレコード値と同じ読み取り、更新、バイト列の往復境界をカバーします。

上流のSeoulTech Korea Maps EUC-KRフィクスチャも、韓国語のフィールド名、16件の実データ、legacy dBASEの`0x4e` driver IDをカバーします。
読み取り、更新、バイト列の往復も検証します。

同じ出典には、4つの韓国語フィールド名と232件の実データを持つ市区町村単位のEUC-KRフィクスチャもあります。
同じ読み取り、更新、バイト列の往復境界を検証します。

クラシックなフィールド記述子名にも、文字値と同じ実効コーデックを使います。
そのため、CJKの列名をJSONキーと更新対象としてそのまま使えます。
記述子の幅制限はバイト単位のままで、XBF出力では引き続きASCIIのフィールド名だけを受け付けます。

## 5. 永続化と復旧

memoポインターとサイドカーのペイロードはクラッシュ後にも一致しなければならないため、DBF互換性は更新境界と結び付きます。

txBASEはWALで次のレコードを使います。

| レコード | 役割 |
| --- | --- |
| `TXOP` | 永続的な HTTP 更新意図 |
| `TXTI` | 正の DBF WAL コミット ID |
| `TXDP` | 完全置換より小さい場合のバイト範囲差分 |
| `TXDB` | 完全な DBF スナップショット |
| `TXDM` | 完全な DBF と memo のスナップショット |
| `TXCD` | DBFトランザクションとともに準備する、単一テーブルのコミット済み行変更イベント |
| `TXCC` | カタログジャーナルとともに準備する、複数テーブルのコミット済み行変更イベント |

DBFまたはmemoサイドカーを置き換える前にWALを同期します。

WAL付き単一テーブルコミットは正のコミットIDを`TXTI` WALレコードと`*.txbase.state`サイドカーに保存します。

復旧はWALをクリアする前にそのサイドカーを書き込みます。

従来のDBFは、最初のWAL付き保存でコミットID 1から始まります。

`*.txbase.cdc`サイドカーは、コミット済みの`TXCD`イベントを同じトランザクションID順で保存します。

復旧は、DBF対象とトランザクション状態を復旧した後で準備済みイベントを公開し、同一イベントの再公開を冪等に処理します。

`.txbase.catalog.cdc`サイドカーは、明示的な複数テーブルカタログトランザクションの`TXCC`イベントを保存します。
カタログジャーナルは、DBF、インデックス、MVCC、トランザクション状態の対象と一緒に、このサイドカーを適用またはロールバックします。

起動時の復旧は、すでに適用された対象を受け付け、基底が一致しない差分を拒否し、状態ペイロードがない場合にサポート対象の`TXOP`を再実行します。

パスからロードした古いテーブルは、ロード後にDBF、memo、スキーマ、トランザクション状態のバイト列が変わると拒否します。

これは現在のプロトタイプの復旧可能性の契約であり、複数writerのレプリケーションプロトコルではありません。

## 6. 保守コマンド

CLIはローカルデータベースの最初の保守境界を公開します。

| コマンド | 動作 |
| --- | --- |
| `txbase schema FILE` | 解析した DBF ヘッダーメタデータとフィールド記述子を JSON で表示する |
| `txbase schema apply FILE SCHEMA_JSON` | メタデータ候補を現在の DBF とアクティブレコードに対して検証し、`FILE`のスキーマサイドカーだけを原子的に置き換える |
| `txbase verify FILE` | DBF をロードし、検出した memo データと`.txidx`サイドカーがあれば検証し、シリアライズ済み DBF を再解析してレコード境界を検査する |
| `txbase pack FILE` | 論理削除したレコードを取り除き、残りの物理レコードを振り直し、使用中の DBT/FPT memo ブロックを圧縮し、既存のインデックスサイドカーを更新して、DBFとmemoのスナップショットを既存WALで永続化する |
| `txbase recall FILE RECORD` | 既存 WAL で一つの論理削除レコードを復元する |
| `txbase cdc FILE [--after TRANSACTION_ID]` | CDCサイドカーから単一テーブルのコミット済み行変更イベントを読み取り、任意で排他的なトランザクションIDカーソル以後に絞り込む |
| `txbase cdc catalog DIRECTORY [--after TRANSACTION_ID]` | カタログCDCサイドカーから原子的な複数テーブルの行変更イベントを読み取り、任意で排他的なカタログトランザクションIDカーソル以後に絞り込む |
| `txbase backup SOURCE DEST` | `SOURCE`を検証し、DBF、検出した`.dbt`または`.fpt`、スキーマ、CDC、有効な`.txidx`サイドカーをコピーする |
| `txbase restore SOURCE DEST` | バックアップを`SOURCE`として同じ検証済みコピー手順を使う |
| `txbase mvcc row FILE RECORD` | 正の物理DBFレコード番号一つについて、保持中の行バージョンを表示する |
| `txbase mvcc row-at FILE TRANSACTION_ID EPOCH RECORD` | commit済みテーブルトランザクション、行epoch、正の物理レコード番号で、保持中の行バージョン一つを読み取る |
| `txbase mvcc gc FILE --keep COUNT [--keep-rows COUNT]` | 完全イメージのうち新しい正の件数を保持する。任意の`--keep-rows`は、最も古い保持対象より前の物理行ごとの古いバージョンを保持し、同期済み一時ファイルを通してMVCC履歴サイドカーだけを置き換える |
| `txbase mvcc catalog gc DIRECTORY --keep COUNT` | 完全イメージのうち新しい正の件数を保持し、同期済み一時ファイルを通してカタログMVCC履歴サイドカーだけを置き換える |
| `txbase wal inspect WAL` | WAL を作成も切り詰めもせずに読み取り、完全なレコードの LSN とペイロード長を表示し、切断された末尾を示す |

コピー操作は、同期済みの一時ファイルを通して各宛先ファイルを置き換えます。

コピー操作はソースを読む前に宛先とソースの保留中のWALまたはスキーマエクスポートを復旧し、その後、DBFと対応するすべてのサイドカーを読み取り、検証し、置き換える間、両方のテーブルロックを保持します。逆方向のコピーがデッドロックしないよう、ロックパスの安定した順序で取得します。

DBFとmemoのサイドカー置換はファイル操作の列であり、新しい複数ファイルトランザクションプロトコルではありません。

そのためコピーが中断された場合は、宛先を使う前に`txbase verify DEST`を実行します。

`PACK`は残存レコードが参照するmemoブロックだけでmemoサイドカーを書き換えます。
残存するDBFのmemoポインターを更新し、既存のインデックスサイドカーも同じWAL付きコミットで更新します。
DBF、圧縮後のmemoスナップショット、インデックススナップショット、MVCCイメージ、トランザクション状態、CDCイベントは、同じ復旧境界を共有します。
memoサイドカーをロードしたメモリ内の`PACK`は、サイドカーの置換を永続化するために`save_with_wal`を使わなければなりません。

`wal inspect`はWALのファイルサイズ、有効なバイト境界、完全なレコードのLSNとペイロード長を表示します。
切断された末尾ヘッダーまたはペイロードは`truncated_tail: true`として表示し、完全な壊れたレコードは引き続きエラーにします。
このコマンドは読み取り専用で、WALを作成せず、切り詰めません。

`PACK`は残存レコードの論理的なmemo値を変更しませんが、物理memoブロック番号を振り直します。

## 7. 意図的な制限

現在のDBF層は次を実装していません。

- 本番規模のセカンダリインデックス保守、完全な選択性コストモデル、より広い複合インデックス計画。
- OLEの意味論と任意の外部memo形式。
- Visual FoxProの式またはコマンドの完全な互換性。
- 複数writerの自動マージと再試行。
- 対応済みのICU4X日本語、中国語、韓国語モード以外のロケール照合と、完全なロケール固有順序の互換性。

これらを追加する前に、契約、外部フィクスチャ、失敗テスト、明確な所有境界が必要です。

## 主な参照先

- [dBASE Level 7 file format](https://www.dbase.com/Knowledgebase/INT/db7_file_fmt.htm)
- [Visual FoxPro table file structure](https://techshelps.github.io/MSDN/FOXHELP/html/contable_file_structure_lp.dbfrp.htm)
- [Visual FoxPro field descriptors and variable-length fields](https://vfphelp.com/help/html/465e7a94-51b7-4e0c-98f9-432864fe5bcc.htm)
- [Visual FoxPro memo file structure](https://vfphelp.com/help/html/74f53aef-fd56-4f1a-a413-4f045922db21.htm)
- [Visual FoxPro auto-increment fields](https://www.vfphelp.com/vfp9/html/bd6eff0c-2ce5-43b7-ab29-f5360cd2f90e.htm)
- [Visual FoxPro code pages](https://www.vfphelp.com/help/html/a3d7b0e0-8320-44b1-8983-17c30a78c6c4.htm)
- [`encoding_rs` encoding and error behavior](https://docs.rs/encoding_rs/latest/encoding_rs/struct.Encoding.html)
- [Unicode Standard Annex #15: Unicode Normalization Forms](https://www.unicode.org/reports/tr15/)
