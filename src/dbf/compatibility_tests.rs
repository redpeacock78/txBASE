use super::*;
use std::fs;

fn decode_hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .flat_map(|token| {
            assert!(token.len() % 2 == 0, "hex fixture token has odd length");
            token
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        })
        .collect()
}

#[test]
fn reads_and_writes_a_pinned_external_foxpro_fixture() {
    let path =
        std::env::temp_dir().join(format!("txbase-external-foxpro-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let lock_path = path.with_extension("txbase.lock");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&wal_path);
    fs::write(
        &path,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/external-foxpro-test.dbf.hex"
        )),
    )
    .unwrap();
    fs::write(
        &memo_path,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/external-foxpro-test.fpt.hex"
        )),
    )
    .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.header.version, 0x30);
    assert_eq!(table.records().len(), 4);
    assert_eq!(table.active_json().len(), 3);
    assert_eq!(table.active_record(1).unwrap().values["ID"], 1);
    assert_eq!(table.active_record(1).unwrap().values["COMP_NAME"], "TEST");
    assert_eq!(
        table.active_record(1).unwrap().values["MELDING"],
        "Message line 1\r\nMessage line 2",
    );
    assert!(table.records()[1].deleted);
    assert_eq!(table.active_record(3).unwrap().values["COMP_NAME"], "TEST2");

    table
        .patch_record(
            1,
            serde_json::json!({"MELDING": "compatibility write"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["MELDING"],
        "compatibility write",
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn reads_and_writes_a_pinned_external_dbase4_fixture() {
    let path =
        std::env::temp_dir().join(format!("txbase-external-dbase4-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let lock_path = path.with_extension("txbase.lock");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&wal_path);
    fs::write(
        &path,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/external-dbase4-test.dbf.hex"
        )),
    )
    .unwrap();
    fs::write(
        &memo_path,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/external-dbase4-test.dbt.hex"
        )),
    )
    .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.header.version, 0x8b);
    assert_eq!(table.records().len(), 10);
    assert_eq!(table.active_json().len(), 10);
    assert_eq!(table.active_record(1).unwrap().values["CHARACTER"], "One");
    assert_eq!(table.active_record(1).unwrap().values["DATE"], "19700101");
    assert_eq!(table.active_record(1).unwrap().values["LOGICAL"], true);
    assert_eq!(
        table.active_record(1).unwrap().values["MEMO"],
        "First memo\r\n"
    );
    assert_eq!(
        table.active_record(2).unwrap().values["MEMO"],
        "Second memo"
    );

    table
        .patch_record(
            2,
            serde_json::json!({"MEMO": "compatibility write"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(2).unwrap().values["MEMO"],
        "compatibility write"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn reads_and_writes_a_pinned_external_dbase3_fixture() {
    let path =
        std::env::temp_dir().join(format!("txbase-external-dbase3-{}.dbf", std::process::id()));
    let lock_path = path.with_extension("txbase.lock");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&wal_path);
    fs::write(
        &path,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/external-dbase3-test.dbf.hex"
        )),
    )
    .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.header.version, 0x03);
    assert_eq!(table.records().len(), 3);
    assert_eq!(table.active_json().len(), 3);
    assert_eq!(table.active_record(1).unwrap().values["TESTBOOL"], true);
    assert_eq!(table.active_record(1).unwrap().values["TESTTEXT"], "test0");
    assert_eq!(
        table.active_record(1).unwrap().values["TESTDATE"],
        "20180101"
    );
    assert_eq!(table.active_record(1).unwrap().values["TESTNUM"], 42);
    assert_eq!(table.active_record(1).unwrap().values["TESTFLOAT"], 42.01);

    table
        .patch_record(
            1,
            serde_json::json!({"TESTTEXT": "rewritten"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["TESTTEXT"],
        "rewritten"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}

#[test]
fn reads_and_writes_a_pinned_external_cp1251_fixture() {
    let path =
        std::env::temp_dir().join(format!("txbase-external-cp1251-{}.dbf", std::process::id()));
    let lock_path = path.with_extension("txbase.lock");
    let wal_path = path.with_extension("txbase.wal");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&lock_path);
    let _ = fs::remove_file(&wal_path);
    fs::write(
        &path,
        decode_hex_fixture(include_str!(
            "../../tests/fixtures/external-cp1251-test.dbf.hex"
        )),
    )
    .unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.header.version, 0x30);
    assert_eq!(table.header.language_driver, 0xc9);
    assert_eq!(table.records().len(), 4);
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "амбулаторно-поликлиническое"
    );
    assert_eq!(table.active_record(2).unwrap().values["NAME"], "больничное");
    assert_eq!(table.active_record(3).unwrap().values["NAME"], "НИИ");
    assert_eq!(
        table.active_record(4).unwrap().values["NAME"],
        "образовательное медицинское учреждение"
    );

    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "перезаписано"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["NAME"],
        "перезаписано"
    );
    assert!(!wal_path.exists());

    fs::remove_file(path).unwrap();
    fs::remove_file(lock_path).unwrap();
}
