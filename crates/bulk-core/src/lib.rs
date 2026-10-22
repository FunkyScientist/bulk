//! Scanning engine and file tree model for Bulk. Has no UI dependencies so it can
//! back the desktop app, the elevated helper and a future CLI.

pub mod helper;
pub mod layout;
pub mod scan;
pub mod tree;
pub mod volume;
mod wide;
pub mod wire;

pub use tree::{Node, NodeId, Tree};
