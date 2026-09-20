# Compatibility fixtures

external-foxpro-test.dbf.hex and external-foxpro-test.fpt.hex are
whitespace-separated hex encodings of the upstream TEST.DBF and TEST.FPT
files from the independent go-foxpro-dbf reader test data.

The source is pinned to commit
3583ae3707e17f815695333443a457b5c7c6c7dc:
https://github.com/SebastiaanKlippert/go-foxpro-dbf/tree/3583ae3707e17f815695333443a457b5c7c6c7dc/testdata
and is distributed under the upstream MIT license.

external-dbase4-test.dbf.hex and external-dbase4-test.dbt.hex are
whitespace-separated hex encodings of the upstream dbase_8b.dbf and
dbase_8b.dbt files from the independent Ruby dbf reader fixtures.

The source is pinned to commit
6b6547384439fd009815d20112b22c58eee83503:
https://github.com/infused/dbf/tree/6b6547384439fd009815d20112b22c58eee83503/spec/fixtures
and is distributed under the upstream MIT license.

external-dbase3-test.dbf.hex is the upstream validFile.dbf fixture from the
independent LindsayBradford/go-dbf reader and writer.

The source is pinned to commit
133325662f853ba7e7ad4676f7633adbd7a41b27:
https://github.com/LindsayBradford/go-dbf/tree/133325662f853ba7e7ad4676f7633adbd7a41b27/testdata
and is distributed under the upstream Apache-2.0 license.

external-cp1251-test.dbf.hex is the upstream cp1251.dbf Visual FoxPro fixture
from the independent Ruby dbf reader fixtures. It exercises real Windows-1251
record values rather than a generated one-byte probe.

The source is pinned to commit
6b6547384439fd009815d20112b22c58eee83503:
https://github.com/infused/dbf/blob/6b6547384439fd009815d20112b22c58eee83503/spec/fixtures/cp1251.dbf
and is distributed under the upstream MIT license.

The `cjk-cp932.dbf.hex`, `cjk-gbk.dbf.hex`, `cjk-euc-kr.dbf.hex`, and
`cjk-big5.dbf.hex` files are pinned byte fixtures for the four Visual FoxPro
CJK language-driver identifiers. They keep the encoded text in the DBF record
area so the driver declaration, byte decoding, byte-width validation, and
round-trip write remain independently reproducible.

`cjk-explicit-codecs.json` contains pinned record-field bytes for all eight
supported explicit CJK codec names and their decoded values. The DBF test
injects each byte sequence into the same record shape before reading and
writing it through the selected override.

`cjk-field-name-gbk.dbf.hex` is a generated minimal Visual FoxPro-shaped DBF
whose field descriptor name and record value are both GBK-encoded. It proves
that CJK descriptor names use the effective codec and remain usable for JSON
reads and mutations. The descriptor name is intentionally short enough for the
classic byte-width limit.

`external-javadbf-gbk.dbf.hex` is a whitespace-separated hex encoding of the
upstream `gbk.dbf` fixture from JavaDBF. It contains three GBK-encoded CJK field
names and 28 real records. The source is pinned to commit
`9d739eb434f48a1d2711e5783932389a813a2808`:
https://github.com/albfernandez/javadbf/tree/9d739eb434f48a1d2711e5783932389a813a2808

The upstream repository is identified as LGPL-3.0; see the
[GNU LGPL v3](https://www.gnu.org/licenses/lgpl-3.0.html). This repository
includes only the fixture bytes as third-party test data, not JavaDBF source
code. The fixture SHA-256 is
`661e3f5c7281fae001fa91245df7281d241200169634939ef7ef97123b35d9bc`.

`external-dbase-rs-cp936.dbf.hex` is a whitespace-separated hex encoding of the
upstream `cp936.dbf` fixture from the independent Rust `dbase-rs` reader and
writer. It contains one CP936/GBK `TEST` field with the value `测试中文`. The
source is pinned to commit
`395af3243cec931f0e9af402f0001b180ec527c1`:
https://github.com/tmontaigu/dbase-rs/tree/395af3243cec931f0e9af402f0001b180ec527c1/tests/data
and is distributed under the upstream MIT license. The fixture SHA-256 is
`674d2b5724d62314e670130f9feca49e46c2c677e7ec4c620e8b957c3799ec92`.

`external-korea-maps-euc-kr.dbf.hex` is a whitespace-separated hex encoding of
the upstream `state.euc_kr.dbf` fixture from SeoulTech/korea-maps. It contains
16 Korean administrative-region records, Korean field names, and the legacy
dBASE `0x4e` driver ID. The source is pinned to commit
`6d002c4fefad4e1d69a21a5ae64b7ccc521f86fe`:
https://github.com/SeoulTech/korea-maps/tree/6d002c4fefad4e1d69a21a5ae64b7ccc521f86fe/shp
The upstream README states that the data comes from Korea Statistics and that
the repository is distributed under the Eclipse Public License. This repository
includes only fixture bytes as third-party test data. The fixture SHA-256 is
`01560d793acc1a12b6bb44a82dc0e4c0dd3a4e7d9ea5fedf37fffeb6240ce990`.

`external-korea-maps-city-euc-kr.dbf.hex` is a whitespace-separated hex encoding
of the upstream `city.euc_kr.dbf` fixture from SeoulTech/korea-maps. It contains
232 Korean administrative-region records, four Korean field names, and the legacy
dBASE `0x4e` driver ID. It is pinned to the same source commit as the state fixture:
`6d002c4fefad4e1d69a21a5ae64b7ccc521f86fe`:
https://github.com/SeoulTech/korea-maps/tree/6d002c4fefad4e1d69a21a5ae64b7ccc521f86fe/shp
This repository includes only fixture bytes as third-party test data. The fixture
SHA-256 is `9b521b581bea2673ab72995551108a16298539fa8ae5fd00a4191d591ade96d5`.
