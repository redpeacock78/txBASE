use super::*;
use crate::index::{IndexDefinition, IndexFile};
use std::fs;

fn table_with_locale_names(language_driver: u8, earlier: &str, later: &str) -> DbfTable {
    let mut bytes = include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect::<Vec<_>>();
    bytes[29] = language_driver;
    bytes[179] = b' ';

    let mut table = DbfTable::from_bytes(&bytes).unwrap();
    for (record, name) in [(1, later), (2, earlier)] {
        table
            .patch_record(
                record,
                serde_json::json!({"NAME": name})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .unwrap();
    }
    table
}

#[test]
fn locale_collation_orders_dbf_rows_through_query_index_and_keyset_cursor() {
    for (language_driver, collation, collation_name, earlier, later) in [
        (0x7b, Collation::Icu4x211Ja, "icu4x-2.1.1-ja", "あい", "愛"),
        (0x7a, Collation::Icu4x211Zh, "icu4x-2.1.1-zh", "艾", "佰"),
        (0x79, Collation::Icu4x211Ko, "icu4x-2.1.1-ko", "가", "伽"),
    ] {
        let table = table_with_locale_names(language_driver, earlier, later);
        let request = parse(
            serde_json::json!({
                "sort": {"NAME": 1},
                "collation": collation_name,
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let scanned = execute_query(&table, &request).unwrap();
        assert_eq!(scanned.len(), 2);
        assert_eq!(scanned[0]["NAME"], earlier);
        assert_eq!(scanned[1]["NAME"], later);

        let path = std::env::temp_dir().join(format!(
            "txbase-locale-collation-{language_driver:02x}-{}.dbf",
            std::process::id()
        ));
        let sidecar = crate::index::sidecar_path(&path);
        let lock = path.with_extension("txbase.lock");
        let wal = path.with_extension("txbase.wal");
        for artifact in [&path, &sidecar, &lock, &wal] {
            let _ = fs::remove_file(artifact);
        }

        fs::write(&path, table.to_bytes()).unwrap();
        IndexFile::build(
            &path,
            vec![IndexDefinition::named("by_name", "NAME").with_collation(collation)],
        )
        .unwrap()
        .save(&path)
        .unwrap();
        assert_eq!(
            explain_query_at(&path, &request).unwrap(),
            QueryPlan::OrderedIndex {
                name: "by_name".into(),
                field: "NAME".into(),
                direction: 1,
            }
        );
        assert_eq!(execute_query_at(&table, &path, &request).unwrap(), scanned);

        let page_request = parse(
            serde_json::json!({
                "page_size": 1,
                "sort": {"NAME": 1},
                "collation": collation_name,
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let first = execute_query_at_page(&table, &path, &page_request).unwrap();
        assert_eq!(first.records[0]["NAME"], earlier);
        let cursor = first.next_cursor.expect("first sorted page has a cursor");
        let next_request = parse(
            serde_json::json!({
                "page_size": 1,
                "sort": {"NAME": 1},
                "collation": collation_name,
                "cursor": cursor,
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let next = execute_query_at_page(&table, &path, &next_request).unwrap();
        assert_eq!(next.records[0]["NAME"], later);
        assert_eq!(next.next_cursor, None);

        for artifact in [&path, &sidecar, &lock, &wal] {
            let _ = fs::remove_file(artifact);
        }
    }
}
