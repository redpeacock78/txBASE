use super::*;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

mod discovery;
mod foreign_keys;
mod transactions;
mod verification;

static NEXT_CATALOG_ID: AtomicUsize = AtomicUsize::new(0);

fn fixture() -> Vec<u8> {
    include_str!("../../tests/fixtures/users.dbf.hex")
        .split_whitespace()
        .map(|token| u8::from_str_radix(token, 16).unwrap())
        .collect()
}

fn temporary_catalog() -> PathBuf {
    let id = NEXT_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
    let root =
        std::env::temp_dir().join(format!("txbase-catalog-test-{}-{id}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).unwrap();
    root
}
