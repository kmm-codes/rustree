//! rustree - Schneller NTFS Disk Space Analyzer
//!
//! Diese Library bietet schnellen Zugriff auf Dateisystem-Informationen
//! durch direktes Lesen der NTFS Master File Table (MFT).
//!
//! # Module
//!
//! - [`mft`] - MFT-Zugriff und Parsing
//! - [`tree`] - Baum-Datenstrukturen
//! - [`treemap`] - Größen als Fläche: Layout und Bild

pub mod mft;
pub mod tree;
pub mod treemap;

/// Prozessweiter Allokator, gilt für alle Binaries, die diese Library
/// einbinden. Parser und Baumaufbau legen Millionen kleiner Strings aus
/// vielen Threads gleichzeitig an; der Windows-Heap serialisiert das,
/// mimalloc hat pro Thread eigene Freilisten.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Re-exports für einfachen Zugriff
pub use mft::{FileEntry, MftError, MftReader};
pub use tree::{TreeBuilder, TreeNode};
