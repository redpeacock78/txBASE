use super::*;
use std::fs;

fn decode_hex_fixture(input: &str) -> Vec<u8> {
    input
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
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
