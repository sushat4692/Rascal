//! Read-only Git engine used by Rascal.
//!
//! The crate deliberately exposes plain data and never hands a
//! [`git2::Repository`] to the UI. All operations here are read-only.

pub mod diff;
pub mod error;
pub mod graph;
pub mod history;
pub mod model;
pub mod repo;
pub mod tree;
pub mod worktree;

pub use error::{Error, Result};
pub use git2::Oid;
pub use graph::{GraphEdge, GraphRow};
pub use model::*;
pub use repo::GitRepo;

/// Maximum number of commits in one history page.
pub const HISTORY_PAGE_SIZE: usize = 300;
/// Maximum number of changed files kept in a commit detail.
pub const MAX_FILES: usize = 2_000;
/// Maximum number of files for which line stats are computed eagerly.
pub const MAX_STAT_FILES: usize = 300;
/// Maximum rendered diff rows per file.
pub const MAX_DIFF_ROWS: usize = 20_000;
/// Maximum bytes read from a blob for display.
pub const MAX_BLOB_BYTES: u64 = 2 * 1024 * 1024;
/// Maximum lines kept in a blob view.
pub const MAX_BLOB_LINES: usize = 50_000;
/// Maximum immediate tree entries.
pub const MAX_TREE_ENTRIES: usize = 5_000;
/// Maximum lanes rendered in the commit graph.
pub const MAX_GRAPH_LANES: usize = 10;
