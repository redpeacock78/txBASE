use super::*;

#[test]
fn decodes_cjk_field_names_with_the_effective_encoding() {
    let bytes = include_str!("../../tests/fixtures/cjk-field-name-gbk.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    let mut table = DbfTable::from_bytes(&bytes).unwrap();

    assert_eq!(table.fields[0].name, "名称");
    assert_eq!(table.active_record(1).unwrap().values["名称"], "中文");

    table
        .patch_record(
            1,
            serde_json::json!({"名称": "中国"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    let reloaded = DbfTable::from_bytes(&table.to_bytes()).unwrap();
    assert_eq!(reloaded.active_record(1).unwrap().values["名称"], "中国");
}
