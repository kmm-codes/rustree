//! Benchmark für Parser und Baumaufbau auf einer MFT-Dump-Datei
//!
//! Den Dump erzeugt ein elevated Lauf der App:
//!
//! ```bash
//! rustree --cli --dump-mft F:\dev\mft-c.bin
//! ```
//!
//! Danach läuft dieses Example ohne Admin-Rechte (Examples bekommen kein
//! Manifest) und misst die Phasen getrennt:
//!
//! ```bash
//! cargo run --release --example scan_bench -- F:\dev\mft-c.bin [Wiederholungen]
//! ```

use rustree::mft::MftReader;
use rustree::tree::{format_size, TreeBuilder};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next().map(PathBuf::from) else {
        eprintln!("Aufruf: scan_bench <dump.bin> [Wiederholungen]");
        std::process::exit(2);
    };
    let repeats: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("Dump: {} ({})", path.display(), format_size(size));

    for run in 1..=repeats {
        let scan_started = Instant::now();
        let entries = match MftReader::scan_dump(&path, |_, _| {}) {
            Ok(entries) => entries,
            Err(e) => {
                eprintln!("Fehler: {}", e);
                std::process::exit(1);
            }
        };
        let scan = scan_started.elapsed();
        let entry_count = entries.len();

        let tree_started = Instant::now();
        let tree = TreeBuilder::new(entries).build();
        let tree_time = tree_started.elapsed();

        println!(
            "Lauf {}: Scan {:.2} s ({} Einträge, {:.1} MB/s), Baum {:.2} s ({} Dateien, {} Ordner, {})",
            run,
            scan.as_secs_f64(),
            entry_count,
            size as f64 / 1e6 / scan.as_secs_f64().max(1e-9),
            tree_time.as_secs_f64(),
            tree.file_count,
            tree.dir_count,
            format_size(tree.total_size)
        );
    }
}
