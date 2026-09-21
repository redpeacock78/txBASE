mod filesystem;
mod memory;
mod object_store;
mod store;

pub use filesystem::FilesystemObjectStore;
pub use memory::MemoryObjectStore;
pub use object_store::{CommitResult, Manifest, ObjectTable};
pub use store::{ObjectStore, ObjectStoreError};

#[cfg(test)]
mod tests;
