#[cfg(not(target_arch = "wasm32"))]
pub mod catalog;
pub mod dbf;
pub mod edge;
pub mod index;
pub mod query;
#[cfg(not(target_arch = "wasm32"))]
pub mod replication;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
#[cfg(not(target_arch = "wasm32"))]
pub mod storage;
pub mod transaction;
pub mod wasm;
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
pub mod wasm_edge;
pub mod xbase;
pub mod xbf;

mod collation;
mod json_order;
mod query_path;

pub use collation::Collation;

/// Maximum size of a JSON request accepted by public parser boundaries.
pub const MAX_JSON_INPUT_BYTES: usize = 1024 * 1024;
