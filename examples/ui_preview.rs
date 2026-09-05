//! UI-Vorschau mit Beispieldaten
//!
//! Zeigt das Hauptfenster mit einem festen Beispielbaum, ohne MFT-Scan und
//! ohne Admin-Rechte: Examples bekommen im Gegensatz zum Binary kein
//! Manifest (siehe build.rs). Praktisch, um am Layout zu arbeiten:
//!
//! ```bash
//! cargo run --example ui_preview
//! ```

use slint::{SharedString, VecModel};
use std::rc::Rc;

slint::include_modules!();

fn entry(name: &str, size: &str, size_bytes: f32, is_directory: bool, depth: i32) -> TreeEntry {
    TreeEntry {
        name: SharedString::from(name),
        size: SharedString::from(size),
        size_bytes,
        is_directory,
        depth,
        expanded: depth == 0 && is_directory,
        has_children: is_directory,
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;

    let entries = vec![
        entry("Windows", "24.10 GB", 2.41e10, true, 0),
        entry("System32", "9.80 GB", 9.8e9, true, 1),
        entry("WinSxS", "8.20 GB", 8.2e9, true, 1),
        entry("explorer.exe", "4.90 MB", 4.9e6, false, 1),
        entry("Users", "118.40 GB", 1.184e11, true, 0),
        entry("Program Files (x86)", "12.30 GB", 1.23e10, true, 0),
        entry("pagefile.sys", "4.00 GB", 4.0e9, false, 0),
        entry("swapfile.sys", "16.00 MB", 1.6e7, false, 0),
        entry("Dateiname mit Umlauten äöü ß.txt", "1.00 KB", 1024.0, false, 0),
        entry(
            "Ein sehr langer Dateiname, der am rechten Rand abgeschnitten werden muss, damit die Größe sichtbar bleibt.log",
            "12.00 KB",
            12288.0,
            false,
            0,
        ),
    ];

    window.set_tree_entries(Rc::new(VecModel::from(entries)).into());
    window.set_status_text(SharedString::from("Vorschau - Beispieldaten"));
    window.run()
}
