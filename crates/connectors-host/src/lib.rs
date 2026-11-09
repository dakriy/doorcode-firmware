//! Connector implementations for a desktop host. Each one is as thin as the
//! external API allows and makes no decisions.

mod file_storage;

pub use file_storage::FileStorage;
