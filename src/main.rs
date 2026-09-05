//! rustree - Schneller NTFS Disk Space Analyzer
//!
//! Dieses Programm nutzt die NTFS Master File Table (MFT) um
//! blitzschnell die Speicherbelegung auf Windows-Laufwerken zu analysieren.

// GUI-Subsystem: beim Start aus dem Startmenü geht kein Konsolenfenster auf.
// Ausgaben für --cli, --help und Fehler holen wir uns in main() über die
// Konsole des Aufrufers zurück, siehe attach_parent_console().
#![windows_subsystem = "windows"]

use clap::Parser;
use slint::SharedString;
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

mod mft;
mod tree;

use mft::MftReader;
use tree::{format_size, TreeBuilder, TreeNode};

// Slint UI einbinden (wird von build.rs kompiliert)
slint::include_modules!();

/// rustree - Schneller NTFS Disk Space Analyzer
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Laufwerk zum Scannen (z.B. C:)
    #[arg(short, long)]
    drive: Option<String>,

    /// CLI-Modus ohne GUI
    #[arg(long)]
    cli: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vor dem Parsen, damit auch clap (--help, Fehlermeldungen) ein Ziel hat
    #[cfg(target_os = "windows")]
    attach_parent_console();

    let args = Args::parse();

    if args.cli {
        run_cli(args.drive)?;
    } else {
        run_gui()?;
    }

    Ok(())
}

/// Hängt den Prozess an die Konsole des Elternprozesses, falls es eine gibt.
///
/// Die EXE ist ein Windows-GUI-Programm (`windows_subsystem = "windows"`) und
/// bekommt deshalb keine eigene Konsole. Aus einem Terminal gestartet sollen
/// `--cli`, `--help` und Fehlermeldungen trotzdem dort erscheinen, also holen
/// wir uns die Konsole des Aufrufers. Ohne Terminal (Explorer, Startmenü)
/// schlägt der Aufruf fehl - das ist der Normalfall für die GUI und in Ordnung.
///
/// Einschränkung: fordert UAC die Admin-Rechte an, startet der AppInfo-Dienst
/// den Prozess, nicht das Terminal. Dann gibt es keine Eltern-Konsole; aus
/// einem Administrator-Terminal heraus klappt es.
#[cfg(target_os = "windows")]
fn attach_parent_console() {
    use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

    // SAFETY: reiner Win32-Aufruf ohne Zeiger; ein Fehlschlag ist erlaubt.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Globaler State für Thread-Sicherheit
struct AppState {
    tree: Option<TreeNode>,
    expanded_paths: HashSet<String>,
}

/// Startet die grafische Benutzeroberfläche
fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    let main_window = MainWindow::new()?;

    // Verfügbare Laufwerke ermitteln
    let drives = get_available_drives();
    let drive_model: Rc<slint::VecModel<SharedString>> =
        Rc::new(slint::VecModel::from(
            drives
                .iter()
                .map(|s| SharedString::from(s.as_str()))
                .collect::<Vec<_>>(),
        ));
    main_window.set_available_drives(drive_model.into());

    // Thread-sicherer Speicher für den aktuellen Baum
    let app_state: Arc<Mutex<AppState>> = Arc::new(Mutex::new(AppState {
        tree: None,
        expanded_paths: HashSet::new(),
    }));

    // Scan-Callback mit Background-Thread
    let window_weak = main_window.as_weak();
    let state_clone = app_state.clone();

    main_window.on_scan_drive(move |drive| {
        let window = window_weak.unwrap();
        let drive_str = drive.to_string();
        let window_weak_thread = window.as_weak();
        let state_for_thread = state_clone.clone();

        window.set_is_scanning(true);
        window.set_status_text(SharedString::from(format!("Initialisiere {}...", drive)));
        window.set_scan_progress(0.0);

        // MFT-Scan in Background-Thread durchführen
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = perform_scan_threaded(&drive_str, window_weak_thread.clone());
            let elapsed = started.elapsed();

            // Ergebnis zurück an UI-Thread senden
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = window_weak_thread.upgrade() {
                    match result {
                        Ok(root_node) => {
                            // Baum speichern und GUI aktualisieren
                            let mut state = state_for_thread.lock().unwrap();
                            state.expanded_paths.clear();

                            let entries = tree_to_entries(&root_node, &state.expanded_paths, 0);
                            let file_count = root_node.file_count;
                            let dir_count = root_node.dir_count;

                            state.tree = Some(root_node);
                            drop(state);

                            let entries_model: Rc<slint::VecModel<TreeEntry>> =
                                Rc::new(slint::VecModel::from(entries));
                            window.set_tree_entries(entries_model.into());

                            window.set_status_text(SharedString::from(format!(
                                "Fertig in {} - {} Dateien, {} Ordner",
                                format_duration(elapsed),
                                file_count,
                                dir_count
                            )));
                        }
                        Err(e) => {
                            window.set_status_text(SharedString::from(format!("Fehler: {}", e)));
                        }
                    }

                    window.set_is_scanning(false);
                    window.set_scan_progress(1.0);
                }
            });
        });
    });

    // Toggle-Callback für Baumansicht
    let window_weak = main_window.as_weak();
    let state_clone = app_state.clone();

    main_window.on_toggle_entry(move |index| {
        let window = window_weak.unwrap();
        let mut state = state_clone.lock().unwrap();

        // Tree klonen um borrow-Konflikte zu vermeiden
        let tree_clone = state.tree.clone();

        if let Some(ref tree) = tree_clone {
            // Finde den Pfad des geklickten Eintrags anhand des Index
            let paths = collect_visible_paths(tree, &state.expanded_paths, 0);

            if let Some(clicked_path) = paths.get(index as usize) {
                let clicked_path = clicked_path.clone();

                // Toggle expanded state
                if state.expanded_paths.contains(&clicked_path) {
                    state.expanded_paths.remove(&clicked_path);
                } else {
                    state.expanded_paths.insert(clicked_path);
                }

                // GUI aktualisieren
                let new_entries = tree_to_entries(tree, &state.expanded_paths, 0);
                drop(state);

                let entries_model: Rc<slint::VecModel<TreeEntry>> =
                    Rc::new(slint::VecModel::from(new_entries));
                window.set_tree_entries(entries_model.into());
            }
        }
    });

    main_window.run()?;
    Ok(())
}

/// Führt den MFT-Scan in einem Background-Thread durch
fn perform_scan_threaded(drive: &str, window_weak: slint::Weak<MainWindow>) -> Result<TreeNode, String> {
    // MFT-Reader erstellen
    let reader = MftReader::new(drive).map_err(|e| format!("{}", e))?;

    // Scan durchführen mit Progress-Updates via invoke_from_event_loop
    let entries = reader
        .scan(|progress, status| {
            let window_weak_clone = window_weak.clone();
            let status_string = status.to_string();
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(w) = window_weak_clone.upgrade() {
                    w.set_scan_progress(progress);
                    w.set_status_text(SharedString::from(status_string));
                }
            });
        })
        .map_err(|e| format!("{}", e))?;

    if entries.is_empty() {
        return Err("Keine Dateien gefunden. Admin-Rechte erforderlich!".to_string());
    }

    // Baum aufbauen
    let builder = TreeBuilder::new(entries);
    let mut tree = builder.build();
    tree.name = drive.to_string();
    tree.path = drive.to_string();

    Ok(tree)
}

/// Formatiert eine Dauer lesbar: "850 ms", "4,2 s", "1 min 12 s"
fn format_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs_f64();
    if secs < 1.0 {
        format!("{} ms", duration.as_millis())
    } else if secs < 60.0 {
        format!("{:.1} s", secs).replace('.', ",")
    } else {
        let whole = duration.as_secs();
        format!("{} min {} s", whole / 60, whole % 60)
    }
}

/// Konvertiert den Baum in flache TreeEntry-Liste für die GUI
fn tree_to_entries(
    node: &TreeNode,
    expanded: &HashSet<String>,
    depth: i32,
) -> Vec<TreeEntry> {
    let mut entries = Vec::new();

    // Nur Root-Kinder anzeigen (nicht den Root selbst)
    if depth == 0 {
        for child in &node.children {
            add_node_to_entries(child, expanded, 0, &mut entries);
        }
    }

    entries
}

/// Sammelt die sichtbaren Pfade in der gleichen Reihenfolge wie tree_to_entries
fn collect_visible_paths(
    node: &TreeNode,
    expanded: &HashSet<String>,
    depth: i32,
) -> Vec<String> {
    let mut paths = Vec::new();

    if depth == 0 {
        for child in &node.children {
            collect_node_paths(child, expanded, 0, &mut paths);
        }
    }

    paths
}

/// Rekursive Hilfsfunktion für collect_visible_paths
fn collect_node_paths(
    node: &TreeNode,
    expanded: &HashSet<String>,
    depth: i32,
    paths: &mut Vec<String>,
) {
    let node_path = if node.path.is_empty() { &node.name } else { &node.path };
    let is_expanded = expanded.contains(node_path);

    paths.push(node_path.clone());

    if is_expanded && node.is_directory {
        for child in &node.children {
            collect_node_paths(child, expanded, depth + 1, paths);
        }
    }
}

/// Rekursive Hilfsfunktion für tree_to_entries
fn add_node_to_entries(
    node: &TreeNode,
    expanded: &HashSet<String>,
    depth: i32,
    entries: &mut Vec<TreeEntry>,
) {
    // Verwende path für expanded-Check, name für Anzeige
    let node_path = if node.path.is_empty() { &node.name } else { &node.path };
    let is_expanded = expanded.contains(node_path);

    entries.push(TreeEntry {
        name: SharedString::from(&node.name),
        size: SharedString::from(format_size(node.total_size)),
        size_bytes: node.total_size as f32,
        is_directory: node.is_directory,
        depth,
        expanded: is_expanded,
        has_children: node.is_directory && !node.children.is_empty(),
    });

    // Kinder hinzufügen wenn expanded
    if is_expanded && node.is_directory {
        for child in &node.children {
            add_node_to_entries(child, expanded, depth + 1, entries);
        }
    }
}

/// CLI-Modus ohne GUI
fn run_cli(drive: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let drive = drive.unwrap_or_else(|| "C:".to_string());

    println!("rustree - NTFS Disk Space Analyzer");
    println!("===================================");
    println!();

    // MFT-Reader erstellen
    println!("Initialisiere {}...", drive);
    let reader = match MftReader::new(&drive) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Fehler: {}", e);
            eprintln!();
            eprintln!("Tipp: Starte das Programm als Administrator!");
            return Ok(());
        }
    };

    println!("{}", reader.info());
    println!();

    // Scan durchführen
    let started = std::time::Instant::now();
    println!("Scanne MFT...");
    let entries = reader.scan(|progress, status| {
        print!("\r{:.0}% - {}", progress * 100.0, status);
        use std::io::Write;
        std::io::stdout().flush().ok();
    })?;

    println!();
    println!();

    if entries.is_empty() {
        eprintln!("Keine Dateien gefunden. Admin-Rechte erforderlich!");
        return Ok(());
    }

    // Baum aufbauen
    println!("Baue Verzeichnisbaum...");
    let builder = TreeBuilder::new(entries);
    let tree = builder.build();

    // Debug: Zeige erste Kinder des Root
    println!();
    println!("Root-Kinder (erste 10):");
    println!("─────────────────────────────────────────────────");
    for (i, child) in tree.children.iter().take(10).enumerate() {
        println!(
            "{:2}. [{}] Name='{}' Path='{}' Size={}",
            i + 1,
            if child.is_directory { "DIR" } else { "FILE" },
            child.name,
            child.path,
            format_size(child.total_size)
        );
    }

    // Top 20 größte Ordner anzeigen
    println!();
    println!("Top 20 größte Ordner/Dateien:");
    println!("─────────────────────────────────────────────────");

    let top_items = TreeBuilder::top_n(&tree, 20);
    for (i, item) in top_items.iter().enumerate() {
        let icon = if item.is_directory { "📁" } else { "📄" };
        println!(
            "{:2}. {} {:>10}  {}",
            i + 1,
            icon,
            format_size(item.total_size),
            if item.path.is_empty() {
                &item.name
            } else {
                &item.path
            }
        );
    }

    println!();
    println!(
        "Gesamt: {} Dateien, {} Ordner",
        tree.file_count, tree.dir_count
    );
    println!("Gesamtgröße: {}", format_size(tree.total_size));
    println!("Fertig in {}", format_duration(started.elapsed()));

    Ok(())
}

/// Ermittelt die verfügbaren Laufwerke auf Windows
fn get_available_drives() -> Vec<String> {
    let mut drives = Vec::new();

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Storage::FileSystem::GetLogicalDrives;

        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26 {
            if mask & (1 << i) != 0 {
                let letter = (b'A' + i) as char;
                drives.push(format!("{}:", letter));
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        drives.push("C:".to_string());
    }

    drives
}
