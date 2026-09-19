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

The `cjk-cp932.dbf.hex`, `cjk-gbk.dbf.hex`, `cjk-euc-kr.dbf.hex`, and
`cjk-big5.dbf.hex` files are pinned byte fixtures for the four Visual FoxPro
CJK language-driver identifiers. They keep the encoded text in the DBF record
area so the driver declaration, byte decoding, byte-width validation, and
round-trip write remain independently reproducible.
