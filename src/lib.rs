//! rustree - Schneller NTFS Disk Space Analyzer
//!
//! Diese Library bietet schnellen Zugriff auf Dateisystem-Informationen
//! durch direktes Lesen der NTFS Master File Table (MFT).
//!
//! # Module
//!
//! - [`mft`] - MFT-Zugriff und Parsing
//! - [`tree`] - Baum-Datenstrukturen

pub mod mft;
pub mod tree;

// Re-exports für einfachen Zugriff
pub use mft::{FileEntry, MftError, MftReader};
pub use tree::{TreeBuilder, TreeNode};
