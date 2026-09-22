use super::cdc::{self, CatalogChangeEvent, CatalogTableChange};
use crate::dbf::{ChangeRecord, ChangeState};
use serde_json::{Map, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn temporary_root() -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-catalog-cdc-test-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    root
}

fn event(transaction_id: u64, id: i64) -> CatalogChangeEvent {
    let mut values = Map::new();
    values.insert("ID".into(), json!(id));
    let mut tables = BTreeMap::new();
    tables.insert(
        "users".into(),
        CatalogTableChange {
            reset: false,
            changes: vec![ChangeRecord {
                record_number: 1,
                before: None,
                after: Some(ChangeState {
                    deleted: false,
                    values,
                }),
            }],
        },
    );
    CatalogChangeEvent {
        transaction_id,
        tables,
    }
}

#[test]
fn catalog_cdc_staging_is_idempotent_and_rejects_conflicting_or_old_events() {
    let root = temporary_root();
    let path = cdc::path_for(&root);
    let first = event(1, 1);
    let first_bytes = cdc::staged_bytes(&path, &first).unwrap();
    fs::write(&path, &first_bytes).unwrap();

    assert_eq!(cdc::staged_bytes(&path, &first).unwrap(), first_bytes);
    assert!(cdc::staged_bytes(&path, &event(1, 2)).is_err());
    assert!(cdc::staged_bytes(&path, &event(0, 0)).is_err());

    let second_bytes = cdc::staged_bytes(&path, &event(2, 2)).unwrap();
    fs::write(&path, second_bytes).unwrap();
    assert!(cdc::staged_bytes(&path, &event(1, 3)).is_err());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn catalog_cdc_reader_repairs_a_torn_final_record() {
    let root = temporary_root();
    let path = cdc::path_for(&root);
    let bytes = cdc::staged_bytes(&path, &event(1, 1)).unwrap();
    let valid_length = bytes.len();
    let mut torn = bytes;
    torn.extend_from_slice(b"TXWL");
    fs::write(&path, torn).unwrap();

    assert_eq!(cdc::read(&root, None).unwrap(), vec![event(1, 1)]);
    assert_eq!(fs::metadata(&path).unwrap().len(), valid_length as u64);

    fs::remove_dir_all(root).unwrap();
}
