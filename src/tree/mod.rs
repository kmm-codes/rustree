//! Tree-Modul - Baut die Verzeichnisstruktur aus MFT-Daten
//!
//! Nach dem MFT-Scan haben wir eine flache Liste mit allen Dateien.
//! Dieses Modul baut daraus eine hierarchische Baumstruktur.

mod builder;
mod node;

pub use builder::TreeBuilder;
pub use node::{format_size, TreeNode};
