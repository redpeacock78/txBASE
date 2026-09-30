#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
#[path = "wasi_query_stream/keys.rs"]
mod keys;

#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
#[path = "wasi_query_stream/wasi_object_store.rs"]
mod wasi_object_store;

#[cfg(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2"))]
mod component {
    use txbase::edge::{AsyncObjectStore, ObjectStoreError};

    use crate::wasi_object_store::WasiFilesystemObjectStore;

    wasip3::cli::command::export!(WasiObjectStoreCheck);

    struct WasiObjectStoreCheck;

    impl wasip3::exports::cli::run::Guest for WasiObjectStoreCheck {
        async fn run() -> Result<(), ()> {
            match check_store().await {
                Ok(()) => Ok(()),
                Err(error) => {
                    eprintln!("{error}");
                    Err(())
                }
            }
        }
    }

    async fn check_store() -> Result<(), String> {
        let store = WasiFilesystemObjectStore::new("/data/wasi-object-store-check")
            .await
            .map_err(|error| error.to_string())?;
        if !matches!(
            WasiFilesystemObjectStore::new("relative-root").await,
            Err(ObjectStoreError::Invalid(_))
        ) {
            return Err("relative object-store root was accepted".into());
        }
        if store
            .get("missing")
            .await
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("missing object unexpectedly exists".into());
        }

        let large = vec![0x5a; 2 * 1024 * 1024];
        store
            .put_if_absent("nested/large.bin", &large)
            .await
            .map_err(|error| error.to_string())?;
        store
            .put_if_absent("nested/large.bin", &large)
            .await
            .map_err(|error| error.to_string())?;
        if !matches!(
            store.put_if_absent("nested/large.bin", b"different").await,
            Err(ObjectStoreError::Conflict(_))
        ) {
            return Err("put-if-absent did not reject different bytes".into());
        }
        if store
            .get("nested/large.bin")
            .await
            .map_err(|error| error.to_string())?
            .as_deref()
            != Some(large.as_slice())
        {
            return Err("streamed object read did not match the written bytes".into());
        }
        if store
            .list("nested/")
            .await
            .map_err(|error| error.to_string())?
            != vec!["nested/large.bin".to_owned()]
        {
            return Err("recursive object listing returned unexpected keys".into());
        }

        store
            .compare_and_swap("nested/large.bin", Some(&large), b"replacement")
            .await
            .map_err(|error| error.to_string())?;
        if !matches!(
            store
                .compare_and_swap("nested/large.bin", Some(&large), b"wrong")
                .await,
            Err(ObjectStoreError::Conflict(_))
        ) {
            return Err("compare-and-swap accepted a stale value".into());
        }
        store
            .compare_and_swap("nested/created.bin", None, b"created")
            .await
            .map_err(|error| error.to_string())?;
        if !matches!(
            store
                .compare_and_swap("nested/created.bin", None, b"duplicate")
                .await,
            Err(ObjectStoreError::Conflict(_))
        ) {
            return Err("compare-and-swap accepted an occupied key".into());
        }
        if !matches!(
            store.get("../outside").await,
            Err(ObjectStoreError::Invalid(_))
        ) {
            return Err("object key traversal was accepted".into());
        }

        store
            .delete("nested/large.bin")
            .await
            .map_err(|error| error.to_string())?;
        store
            .delete("nested/large.bin")
            .await
            .map_err(|error| error.to_string())?;
        store
            .delete("nested/created.bin")
            .await
            .map_err(|error| error.to_string())?;
        if !store
            .list("nested/")
            .await
            .map_err(|error| error.to_string())?
            .is_empty()
        {
            return Err("deleted objects remain in the listing".into());
        }
        Ok(())
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "wasi", target_env = "p2")))]
#[allow(dead_code)]
fn main() {}
