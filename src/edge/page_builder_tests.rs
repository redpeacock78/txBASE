use crate::edge::tests::block_on;
use crate::edge::{MemoryObjectStore, SyncObjectStoreAdapter};

use super::{PAGE_SIZE, make_manifest, make_manifest_async};

#[test]
fn sync_and_async_page_manifests_split_exact_page_boundaries() {
    let store = MemoryObjectStore::new();
    let async_store = SyncObjectStoreAdapter::new(store.clone());

    for size in [PAGE_SIZE, PAGE_SIZE + 1] {
        let snapshot = vec![0x5a; size];
        let synchronous = make_manifest(&store, "users/", 0, None, &snapshot).unwrap();
        let asynchronous = block_on(make_manifest_async(
            &async_store,
            "users/",
            0,
            None,
            &snapshot,
        ))
        .unwrap();

        assert_eq!(asynchronous, synchronous);
        assert_eq!(synchronous.snapshot_length, size as u64);
        assert_eq!(
            synchronous
                .pages
                .iter()
                .map(|page| page.length)
                .collect::<Vec<_>>(),
            if size == PAGE_SIZE {
                vec![size as u32]
            } else {
                vec![PAGE_SIZE as u32, 1]
            }
        );
    }
}
