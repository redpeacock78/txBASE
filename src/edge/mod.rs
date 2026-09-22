#[cfg(not(target_arch = "wasm32"))]
mod filesystem;
mod memory;
mod object_store;
mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use filesystem::FilesystemObjectStore;
pub use memory::MemoryObjectStore;
pub use object_store::{CommitResult, Manifest, ObjectTable};
pub use store::{ObjectStore, ObjectStoreError};

#[cfg(test)]
mod tests;
