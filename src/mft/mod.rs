//! MFT (Master File Table) Modul
//!
//! Dieses Modul ist das Herzstück von rustree. Es liest die NTFS MFT
//! direkt aus, um blitzschnelle Scans zu ermöglichen.
//!
//! # Wie funktioniert die MFT?
//!
//! Die MFT ist eine spezielle Datei auf jeder NTFS-Partition, die
//! Metadaten über ALLE Dateien auf dem Laufwerk enthält:
//! - Dateiname
//! - Größe
//! - Erstellungsdatum
//! - Parent-Verzeichnis (als Referenz-Nummer)
//!
//! Durch direktes Lesen der MFT müssen wir nicht rekursiv durch
//! alle Ordner navigieren - wir bekommen alles in einem Durchgang!

mod reader;
mod parser;
mod types;

pub use reader::MftReader;
pub use parser::MftParser;
pub use types::{FileEntry, MftError};
