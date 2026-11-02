//! Scanning engine and file tree model for Bulk. Has no UI dependencies so it can
//! back the desktop app, the elevated helper and the CLI.

pub mod diff;
pub mod drive;
pub mod dupes;
pub mod export;
pub mod filter;
pub mod helper;
pub mod junk;
pub mod kinds;
pub mod layout;
pub mod os;
pub mod scan;
pub mod search;
pub mod settings;
pub mod snapshot;
pub mod trash;
pub mod tree;
pub mod usn;
pub mod volume;
mod wide;
pub mod wire;

pub use tree::{Node, NodeId, Tree};
