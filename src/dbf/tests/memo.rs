use super::super::*;
use super::fixture;

#[test]
fn reads_and_writes_dbase3_memo_sidecar() {
    let path = std::env::temp_dir().join(format!("txbase-dbase3-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo from dbt";
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + text.len()].copy_from_slice(text);
    memo[DBT_BLOCK_SIZE + text.len()] = EOF_MARKER;
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "memo from dbt"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "memo from dbt"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "new memo"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table
        .patch_record(
            1,
            serde_json::json!({"AGE": 30}).as_object().unwrap().clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "new memo");
    assert_eq!(reread.active_record(1).unwrap().values["AGE"], 30);
    assert_eq!(
        &reread.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x83).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), b"new memo");
    assert_eq!(u32::from_be_bytes(memo.bytes[..4].try_into().unwrap()), 3);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_dbase3_binary_sidecar() {
    let path =
        std::env::temp_dir().join(format!("txbase-dbase3-binary-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x83;
    bytes[64 + 11] = b'B';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let binary_start = record_start + 4;
    bytes[binary_start..binary_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    let binary = [0x00, 0x1a, 0xff, 0x7f];
    memo[..4].copy_from_slice(&2u32.to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + binary.len()].copy_from_slice(&binary);
    memo[DBT_BLOCK_SIZE + binary.len()..DBT_BLOCK_SIZE + binary.len() + 2]
        .copy_from_slice(&[EOF_MARKER, EOF_MARKER]);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "001aff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let mut reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[binary_start..binary_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x83).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(u32::from_be_bytes(memo.bytes[..4].try_into().unwrap()), 3);

    let error = reread
        .patch_record(
            1,
            serde_json::json!({"NAME": "001a1aff"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("dBASE III binary data cannot contain its 0x1a1a terminator")
    );
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_dbase4_memo_sidecar() {
    let path = std::env::temp_dir().join(format!("txbase-dbase4-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x8b;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    let text = b"memo from dbase4";
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&[0xff, 0xff, 0x08, 0x00]);
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&((text.len() as u32 + 8).to_le_bytes()));
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + text.len()].copy_from_slice(text);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "memo from dbase4"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "changed dbase4"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["NAME"],
        "changed dbase4"
    );
    assert_eq!(
        &reread.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x8b).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), b"changed dbase4");
    assert_eq!(
        &memo.bytes[DBT_BLOCK_SIZE * 2..DBT_BLOCK_SIZE * 2 + 4],
        &[0xff, 0xff, 0x08, 0x00]
    );

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_dbase4_binary_sidecar() {
    let path =
        std::env::temp_dir().join(format!("txbase-dbase4-binary-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("DBT");
    let lowercase_memo_path = path.with_extension("dbt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);
    let _ = fs::remove_file(&lowercase_memo_path);

    let mut bytes = fixture();
    bytes[0] = 0x8b;
    bytes[64 + 11] = b'B';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let binary_start = record_start + 4;
    bytes[binary_start..binary_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let dbt_block_size = 1024;
    let mut memo = vec![0; dbt_block_size * 2];
    memo[20..22].copy_from_slice(&(dbt_block_size as u16).to_le_bytes());
    let binary = [0x00, 0x1a, 0xff, 0x7f];
    memo[dbt_block_size..dbt_block_size + 4].copy_from_slice(&[0xff, 0xff, 0x08, 0x00]);
    memo[dbt_block_size + 4..dbt_block_size + 8]
        .copy_from_slice(&((binary.len() as u32 + 8).to_le_bytes()));
    memo[dbt_block_size + 8..dbt_block_size + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["NAME"], "001aff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["NAME"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[binary_start..binary_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0x8b).unwrap();
    assert_eq!(memo.block_size, dbt_block_size);
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(u32::from_le_bytes(memo.bytes[..4].try_into().unwrap()), 3);
    let sidecar_count = fs::read_dir(path.parent().unwrap())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|candidate| {
            candidate.file_stem() == path.file_stem()
                && candidate
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dbt"))
        })
        .count();
    assert_eq!(sidecar_count, 1);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_foxpro_fpt_memo_sidecar() {
    let path = std::env::temp_dir().join(format!("txbase-foxpro-memo-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = fixture();
    bytes[0] = 0xf5;
    bytes[64 + 11] = b'M';
    let record_start = usize::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let memo_start = record_start + 4;
    bytes[memo_start..memo_start + 10].copy_from_slice(b"         1");
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&1u32.to_be_bytes());
    let text = b"memo from fpt";
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(text.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + text.len()].copy_from_slice(text);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        table.active_record(1).unwrap().values["NAME"],
        "memo from fpt"
    );
    table
        .patch_record(
            1,
            serde_json::json!({"NAME": "changed fpt"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(
        reread.active_record(1).unwrap().values["NAME"],
        "changed fpt"
    );
    assert_eq!(
        &reread.to_bytes()[memo_start..memo_start + 10],
        b"         2"
    );
    let memo = MemoFile::open(&memo_path, 0xf5).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), b"changed fpt");

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_foxpro_binary_memo_sidecar_as_hex() {
    let path = std::env::temp_dir().join(format!(
        "txbase-foxpro-binary-memo-{}.dbf",
        std::process::id()
    ));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = vec![0; 71];
    bytes[0] = 0xf5;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..37].copy_from_slice(b"BMEMO");
    bytes[43] = b'M';
    bytes[48] = 4;
    bytes[50] = 0x04;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    let pointer_start = 66;
    bytes[pointer_start..pointer_start + 4].copy_from_slice(&1u32.to_le_bytes());
    bytes[70] = EOF_MARKER;
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
    let binary = [0x10, 0x20, 0xf0];
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(binary.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["BMEMO"], "1020f0");
    table
        .patch_record(
            1,
            serde_json::json!({"BMEMO": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["BMEMO"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[pointer_start..pointer_start + 4],
        &2u32.to_le_bytes()
    );
    let memo = MemoFile::open(&memo_path, 0xf5).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn preserves_visual_foxpro_null_sidecar_values() {
    let mut bytes = vec![0; 104];
    bytes[0] = 0x30;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&97u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&6u16.to_le_bytes());
    bytes[32..37].copy_from_slice(b"BMEMO");
    bytes[43] = b'M';
    bytes[48] = 4;
    bytes[50] = 0x06;
    bytes[64..74].copy_from_slice(b"_NullFlags");
    bytes[80] = 1;
    bytes[82] = 0x01;
    bytes[96] = FIELD_TERMINATOR;
    bytes[97] = ACTIVE_RECORD;
    bytes[98..102].copy_from_slice(&1u32.to_le_bytes());
    bytes[102] = 0x01;
    bytes[103] = EOF_MARKER;

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[512..516].copy_from_slice(&0u32.to_be_bytes());
    memo[516..520].copy_from_slice(&3u32.to_be_bytes());
    memo[520..523].copy_from_slice(&[0x10, 0x20, 0xf0]);

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    table
        .resolve_memos(&MemoFile {
            bytes: memo,
            block_size: DBT_BLOCK_SIZE,
            format: MemoFormat::FoxPro,
        })
        .unwrap();
    assert_eq!(table.active_record(1).unwrap().values["BMEMO"], Value::Null);
}

#[test]
fn reads_and_writes_visual_foxpro_picture_fpt_sidecar_as_hex() {
    let path =
        std::env::temp_dir().join(format!("txbase-foxpro-binary-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = vec![0; 71];
    bytes[0] = 0x30;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..37].copy_from_slice(b"IMAGE");
    bytes[43] = b'P';
    bytes[48] = 4;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    let pointer_start = 66;
    bytes[pointer_start..pointer_start + 4].copy_from_slice(&1u32.to_le_bytes());
    bytes[70] = EOF_MARKER;
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
    let binary = [0x00, 0x01, 0xff, 0x7f];
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(binary.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["IMAGE"], "0001ff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"IMAGE": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["IMAGE"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[pointer_start..pointer_start + 4],
        &2u32.to_le_bytes()
    );
    let memo = MemoFile::open(&memo_path, 0x30).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}

#[test]
fn reads_and_writes_visual_foxpro_blob_fpt_sidecar_as_hex() {
    let path = std::env::temp_dir().join(format!("txbase-foxpro-blob-{}.dbf", std::process::id()));
    let memo_path = path.with_extension("fpt");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&memo_path);

    let mut bytes = vec![0; 71];
    bytes[0] = 0x32;
    bytes[4..8].copy_from_slice(&1u32.to_le_bytes());
    bytes[8..10].copy_from_slice(&65u16.to_le_bytes());
    bytes[10..12].copy_from_slice(&5u16.to_le_bytes());
    bytes[32..36].copy_from_slice(b"BLOB");
    bytes[43] = b'W';
    bytes[48] = 4;
    bytes[64] = FIELD_TERMINATOR;
    bytes[65] = ACTIVE_RECORD;
    let pointer_start = 66;
    bytes[pointer_start..pointer_start + 4].copy_from_slice(&1u32.to_le_bytes());
    bytes[70] = EOF_MARKER;
    fs::write(&path, bytes).unwrap();

    let mut memo = vec![0; DBT_BLOCK_SIZE * 2];
    memo[6..8].copy_from_slice(&(DBT_BLOCK_SIZE as u16).to_be_bytes());
    memo[DBT_BLOCK_SIZE..DBT_BLOCK_SIZE + 4].copy_from_slice(&0u32.to_be_bytes());
    let binary = [0x00, 0x01, 0xff, 0x7f];
    memo[DBT_BLOCK_SIZE + 4..DBT_BLOCK_SIZE + 8]
        .copy_from_slice(&(binary.len() as u32).to_be_bytes());
    memo[DBT_BLOCK_SIZE + 8..DBT_BLOCK_SIZE + 8 + binary.len()].copy_from_slice(&binary);
    fs::write(&memo_path, memo).unwrap();

    let mut table = DbfTable::from_path(&path).unwrap();
    assert_eq!(table.active_record(1).unwrap().values["BLOB"], "0001ff7f");
    table
        .patch_record(
            1,
            serde_json::json!({"BLOB": "deadbeef"})
                .as_object()
                .unwrap()
                .clone(),
        )
        .unwrap();
    table.save_with_wal(&path).unwrap();

    let reread = DbfTable::from_path(&path).unwrap();
    assert_eq!(reread.active_record(1).unwrap().values["BLOB"], "deadbeef");
    assert_eq!(
        &reread.to_bytes()[pointer_start..pointer_start + 4],
        &2u32.to_le_bytes()
    );
    let memo = MemoFile::open(&memo_path, 0x32).unwrap();
    assert_eq!(memo.read(2).unwrap().unwrap(), [0xde, 0xad, 0xbe, 0xef]);

    fs::remove_file(path).unwrap();
    fs::remove_file(memo_path).unwrap();
}
