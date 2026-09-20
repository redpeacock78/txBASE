use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

fn temporary_root() -> PathBuf {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "txbase-catalog-transaction-test-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    root
}

fn prepared_journal(root: &Path, phase: Phase) {
    let journal = root.join(JOURNAL_DIR);
    fs::create_dir(&journal).unwrap();
    fs::create_dir(journal.join("before")).unwrap();
    fs::create_dir(journal.join("after")).unwrap();
    write_synced(&journal.join("before/0"), b"users-before").unwrap();
    write_synced(&journal.join("before/1"), b"posts-before").unwrap();
    write_synced(&journal.join("before/2"), &transaction_state_bytes(1)).unwrap();
    write_synced(&journal.join("after/0"), b"users-after").unwrap();
    write_synced(&journal.join("after/1"), b"posts-after").unwrap();
    write_synced(&journal.join("after/2"), &transaction_state_bytes(2)).unwrap();
    write_manifest(
        &journal,
        &Manifest {
            phase,
            changes: vec![
                ManifestChange {
                    target: "users.dbf".into(),
                    before: Some("0".into()),
                    after: Some("0".into()),
                },
                ManifestChange {
                    target: "posts.dbf".into(),
                    before: Some("1".into()),
                    after: Some("1".into()),
                },
                ManifestChange {
                    target: TRANSACTION_STATE.into(),
                    before: Some("2".into()),
                    after: Some("2".into()),
                },
            ],
        },
    )
    .unwrap();
}

#[test]
fn prepared_journal_rolls_back_partial_catalog_commit() {
    let root = temporary_root();
    fs::write(root.join("users.dbf"), b"users-after").unwrap();
    fs::write(root.join("posts.dbf"), b"posts-before").unwrap();
    fs::write(root.join(TRANSACTION_STATE), transaction_state_bytes(2)).unwrap();
    prepared_journal(&root, Phase::Prepared);

    recover(&root).unwrap();

    assert_eq!(fs::read(root.join("users.dbf")).unwrap(), b"users-before");
    assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), b"posts-before");
    assert_eq!(read_transaction_id_locked(&root).unwrap(), Some(1));
    assert!(!root.join(JOURNAL_DIR).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn committed_journal_replays_the_complete_catalog_commit() {
    let root = temporary_root();
    fs::write(root.join("users.dbf"), b"users-before").unwrap();
    fs::write(root.join("posts.dbf"), b"posts-before").unwrap();
    fs::write(root.join(TRANSACTION_STATE), transaction_state_bytes(1)).unwrap();
    prepared_journal(&root, Phase::Committed);

    recover(&root).unwrap();

    assert_eq!(fs::read(root.join("users.dbf")).unwrap(), b"users-after");
    assert_eq!(fs::read(root.join("posts.dbf")).unwrap(), b"posts-after");
    assert_eq!(read_transaction_id_locked(&root).unwrap(), Some(2));
    assert!(!root.join(JOURNAL_DIR).exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_a_malformed_catalog_transaction_state() {
    let root = temporary_root();
    fs::write(root.join(TRANSACTION_STATE), b"not a catalog state").unwrap();

    let error = match read_lock(&root) {
        Ok(_) => panic!("malformed catalog state was accepted"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("catalog transaction state header is invalid")
    );
    fs::remove_dir_all(root).unwrap();
}
