use super::*;

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/external-javadbf-gbk.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn cp936_fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/external-dbase-rs-cp936.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn korea_maps_fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/external-korea-maps-euc-kr.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn korea_maps_city_fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/external-korea-maps-city-euc-kr.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

#[test]
fn reads_and_writes_a_pinned_upstream_gbk_fixture() {
    let bytes = fixture();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    assert_eq!(table.header.version, 0x30);
    assert_eq!(table.header.language_driver, 0x7a);
    assert_eq!(table.records().len(), 28);
    assert_eq!(table.fields[0].name, "设计编号");
    assert_eq!(table.fields[1].name, "审核信息");
    assert_eq!(table.fields[2].name, "是否打印");
    assert_eq!(table.schema_json()["encoding"], "GBK/CP936");
    assert_eq!(
        table.active_record(1).unwrap().values["设计编号"],
        "201500256-059K-001-001牡丹江林口朝阳"
    );
    assert_eq!(
        table.active_record(1).unwrap().values["审核信息"],
        ">经营区：商品林；起源：天然林；采伐类型：抚育伐；采伐方式：透光伐；经营措施：上方透光；经理小班：1"
    );

    table
        .patch_record(
            1,
            serde_json::json!({"设计编号": "测试"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(
        reloaded.active_record(1).unwrap().values["设计编号"],
        "测试"
    );
}

#[test]
fn reads_and_writes_a_pinned_upstream_cp936_fixture() {
    let bytes = cp936_fixture();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    assert_eq!(table.header.version, 0x03);
    assert_eq!(table.header.language_driver, 0x4d);
    assert_eq!(table.records().len(), 1);
    assert_eq!(table.fields[0].name, "TEST");
    assert_eq!(table.schema_json()["encoding"], "GBK/CP936");
    assert_eq!(table.active_record(1).unwrap().values["TEST"], "测试中文");

    table
        .patch_record(
            1,
            serde_json::json!({"TEST": "中文更新"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(
        reloaded.active_record(1).unwrap().values["TEST"],
        "中文更新"
    );
}

#[test]
fn reads_and_writes_a_pinned_upstream_euc_kr_fixture() {
    let bytes = korea_maps_fixture();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    assert_eq!(table.header.version, 0x03);
    assert_eq!(table.header.language_driver, 0x4e);
    assert_eq!(table.records().len(), 16);
    assert_eq!(table.fields[0].name, "광역시코드");
    assert_eq!(table.fields[1].name, "광역시명");
    assert_eq!(table.schema_json()["encoding"], "EUC-KR/CP949");
    assert_eq!(table.active_record(1).unwrap().values["광역시코드"], "11");
    assert_eq!(
        table.active_record(1).unwrap().values["광역시명"],
        "서울특별"
    );

    table
        .patch_record(
            1,
            serde_json::json!({"광역시명": "테스트"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(
        reloaded.active_record(1).unwrap().values["광역시명"],
        "테스트"
    );
}

#[test]
fn reads_and_writes_a_pinned_upstream_city_euc_kr_fixture() {
    let bytes = korea_maps_city_fixture();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    assert_eq!(table.header.version, 0x03);
    assert_eq!(table.header.language_driver, 0x4e);
    assert_eq!(table.records().len(), 232);
    assert_eq!(table.fields[0].name, "광역시코드");
    assert_eq!(table.fields[1].name, "광역시명");
    assert_eq!(table.fields[2].name, "시군구코드");
    assert_eq!(table.fields[3].name, "시군구명");
    assert_eq!(table.schema_json()["encoding"], "EUC-KR/CP949");
    assert_eq!(table.active_record(1).unwrap().values["광역시코드"], "11");
    assert_eq!(
        table.active_record(1).unwrap().values["광역시명"],
        "서울특별"
    );
    assert_eq!(table.active_record(1).unwrap().values["시군구코드"], "1111");
    assert_eq!(table.active_record(1).unwrap().values["시군구명"], "종로구");

    table
        .patch_record(
            1,
            serde_json::json!({"시군구명": "테스트"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(
        reloaded.active_record(1).unwrap().values["시군구명"],
        "테스트"
    );
}
