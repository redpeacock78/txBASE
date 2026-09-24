#[cfg(not(target_arch = "wasm32"))]
mod filesystem;
mod memory;
mod object_store;
mod query_stream;
mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use filesystem::FilesystemObjectStore;
pub use memory::MemoryObjectStore;
pub use object_store::{AsyncObjectTable, CommitResult, Manifest, ObjectTable};
pub use query_stream::AsyncObjectQueryStream;
pub use store::{
    AsyncObjectStore, AsyncObjectStoreFuture, ObjectStore, ObjectStoreError, SyncObjectStoreAdapter,
};

#[cfg(test)]
mod query_stream_tests;
#[cfg(test)]
mod tests;
