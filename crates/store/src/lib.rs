//! Local-first storage: notebooks -> sections -> pages as human-readable Typst plus JSON
//! sidecars, atomic writes, trash, history, backups, optional age encryption.

pub mod backup;
pub mod fsio;
pub mod history;
pub mod model;
mod store;
pub mod templates;
pub mod vault;

pub use fsio::{iso_date, new_id, now_ms, Result, StoreError};
pub use model::*;
pub use store::{NotebookMeta, Store, TrashEntry, TRASH_RETENTION_MS};
pub use vault::{Vault, VaultError};
