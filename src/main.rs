//! rustree - Schneller NTFS Disk Space Analyzer
//!
//! Dieses Programm nutzt die NTFS Master File Table (MFT) um
//! blitzschnell die Speicherbelegung auf Windows-Laufwerken zu analysieren.

// GUI-Subsystem: beim Start aus dem Startmenü geht kein Konsolenfenster auf.
// Ausgaben für --cli, --help und Fehler holen wir uns in main() über die
// Konsole des Aufrufers zurück, siehe attach_parent_console().
#![windows_subsystem = "windows"]

use clap::Parser;
use rustree::mft::MftReader;
use rustree::tree::{format_size, TreeBuilder, TreeNode};
use rustree::treemap::Treemap;
use slint::{Image, Model, Rgb8Pixel, SharedPixelBuffer, SharedString, Timer, TimerMode};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

    /// Rohe MFT-Records beim Scan in diese Datei schreiben (nur mit --cli).
    /// `cargo run --release --example scan_bench -- <DATEI>` liest sie ohne
    /// Admin-Rechte wieder ein, zum Messen von Parser und Baumaufbau.
    #[arg(long, value_name = "DATEI", requires = "cli")]
    dump_mft: Option<std::path::PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Vor dem Parsen, damit auch clap (--help, Fehlermeldungen) ein Ziel hat
    #[cfg(target_os = "windows")]
    attach_parent_console();

    let args = Args::parse();

    if args.cli {
        run_cli(args.drive, args.dump_mft.as_deref())?;
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
    use windows::Win32::System::Console::{
        AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE,
    };

    // SAFETY: reine Win32-Aufrufe ohne Zeiger; ein Fehlschlag ist erlaubt.
    unsafe {
        // Ist die Ausgabe bereits umgeleitet (`rustree --cli > log`), hat der
        // Prozess ein Handle geerbt - das darf AttachConsole nicht ersetzen.
        if GetStdHandle(STD_OUTPUT_HANDLE).is_ok_and(|handle| !handle.is_invalid()) {
            return;
        }
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Zeilenhöhe der Baumansicht, muss zu ui/main.slint passen
const ROW_HEIGHT: f32 = 28.0;

/// Sortierung der Baumansicht: Spalte und Richtung
#[derive(Clone, Copy, PartialEq)]
struct SortOrder {
    column: SortColumn,
    descending: bool,
}

impl SortOrder {
    /// Erste Wahl je Spalte: Größe absteigend (wie der Baum ohnehin
    /// sortiert ist), Namen aufsteigend
    fn default_for(column: SortColumn) -> Self {
        Self {
            column,
            descending: column == SortColumn::Size,
        }
    }
}

/// Globaler State für Thread-Sicherheit
struct AppState {
    tree: Option<TreeNode>,
    /// IDs (MFT-Referenzen) der aufgeklappten Ordner
    expanded: HashSet<u64>,
    sort: SortOrder,
    /// Markierter Knoten (MFT-Referenz), per Klick in Liste oder Treemap
    selected: Option<u64>,
    /// Weg von der Wurzel zu dem Ordner, den die Treemap zeigt
    /// (leer = das ganze Laufwerk)
    treemap_root: Vec<u64>,
    /// Zuletzt gezeichnete Treemap, für Hover und Klick
    treemap: Option<Treemap>,
}

impl AppState {
    fn new() -> Self {
        Self {
            tree: None,
            expanded: HashSet::new(),
            sort: SortOrder::default_for(SortColumn::Size),
            selected: None,
            treemap_root: Vec::new(),
            treemap: None,
        }
    }

    /// Der Ordner, den die Treemap gerade zeigt
    fn treemap_root_node(&self) -> Option<&TreeNode> {
        node_at(self.tree.as_ref()?, &self.treemap_root)
    }
}

/// Alles, was die GUI-Callbacks teilen
#[derive(Clone)]
struct Gui {
    window: slint::Weak<MainWindow>,
    state: Arc<Mutex<AppState>>,
    /// Zähler für Treemap-Renderläufe: nur der jüngste darf sein Bild setzen
    render_generation: Arc<AtomicU64>,
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

    let gui = Gui {
        window: main_window.as_weak(),
        state: Arc::new(Mutex::new(AppState::new())),
        render_generation: Arc::new(AtomicU64::new(0)),
    };

    // Scan-Callback mit Background-Thread
    let gui_scan = gui.clone();
    main_window.on_scan_drive(move |drive| {
        let window = gui_scan.window.unwrap();
        let drive_str = drive.to_string();
        let window_weak_thread = window.as_weak();
        let gui = gui_scan.clone();

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
                        Ok((root_node, timings)) => {
                            let file_count = root_node.file_count;
                            let dir_count = root_node.dir_count;

                            // Neuer Baum, alles andere auf Anfang - bis auf
                            // die gewählte Sortierung
                            let mut state = gui.state.lock().unwrap();
                            let sort = state.sort;
                            *state = AppState::new();
                            state.sort = sort;
                            state.tree = Some(root_node);
                            refresh_list(&window, &state);
                            drop(state);

                            window.set_treemap_root_text(SharedString::default());
                            window.set_treemap_hover_text(SharedString::default());
                            schedule_treemap_render(&gui);

                            window.set_status_text(SharedString::from(format!(
                                "Fertig in {} (Scan {}, Baum {}) - {} Dateien, {} Ordner",
                                format_duration(elapsed),
                                format_duration(timings.scan),
                                format_duration(timings.tree),
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

    // Klick auf eine Zeile: Ordner auf- oder zuklappen, Zeile markieren
    let gui_toggle = gui.clone();
    main_window.on_toggle_entry(move |index| {
        let window = gui_toggle.window.unwrap();
        let mut state = gui_toggle.state.lock().unwrap();

        // Baum und Klapp-Zustand getrennt ausleihen - der Baum wird nur
        // gelesen, nie kopiert (bei Millionen Knoten wäre ein Klon pro
        // Klick spürbar)
        let AppState {
            tree,
            expanded,
            sort,
            selected,
            ..
        } = &mut *state;
        let Some(tree) = tree.as_ref() else {
            return;
        };

        // Finde die ID des geklickten Eintrags anhand des Index
        let visible = collect_visible_ids(tree, expanded, *sort);
        let Some(&clicked) = visible.get(index as usize) else {
            return;
        };

        // Toggle expanded state
        if !expanded.remove(&clicked) {
            expanded.insert(clicked);
        }
        *selected = Some(clicked);

        refresh_list(&window, &state);
    });

    // Klick auf einen Spaltenkopf: Spalte wechseln oder Richtung umdrehen
    let gui_sort = gui.clone();
    main_window.on_sort_by(move |column| {
        let window = gui_sort.window.unwrap();
        let mut state = gui_sort.state.lock().unwrap();
        state.sort = if state.sort.column == column {
            SortOrder {
                column,
                descending: !state.sort.descending,
            }
        } else {
            SortOrder::default_for(column)
        };
        refresh_list(&window, &state);
    });

    // Größenänderung des Treemap-Bereichs: erst zeichnen, wenn Ruhe ist -
    // beim Ziehen des Fensters kommen Dutzende Änderungen pro Sekunde
    let gui_resize = gui.clone();
    let resize_timer = Rc::new(Timer::default());
    main_window.on_treemap_resized(move || {
        let gui = gui_resize.clone();
        resize_timer.start(TimerMode::SingleShot, Duration::from_millis(150), move || {
            schedule_treemap_render(&gui);
        });
    });

    // Maus über der Treemap: Pfad und Größe anzeigen
    let gui_hover = gui.clone();
    main_window.on_treemap_hover(move |x, y| {
        let window = gui_hover.window.unwrap();
        let state = gui_hover.state.lock().unwrap();
        let text = treemap_hit(&window, &state, x, y)
            .and_then(|chain| describe(state.tree.as_ref()?, &chain))
            .unwrap_or_default();
        window.set_treemap_hover_text(SharedString::from(text));
    });

    // Klick in die Treemap: den Eintrag im Baum aufklappen und markieren
    let gui_click = gui.clone();
    main_window.on_treemap_clicked(move |x, y| {
        let window = gui_click.window.unwrap();
        let mut state = gui_click.state.lock().unwrap();
        let Some(chain) = treemap_hit(&window, &state, x, y) else {
            return;
        };
        let Some((&target, ancestors)) = chain.split_last() else {
            return;
        };
        state.expanded.extend(ancestors.iter().copied());
        state.selected = Some(target);
        refresh_list(&window, &state);
        scroll_to_selected(&window);
    });

    // Doppelklick: in den Ordner unter der Maus absteigen
    let gui_descend = gui.clone();
    main_window.on_treemap_descend(move |x, y| {
        let window = gui_descend.window.unwrap();
        let mut state = gui_descend.state.lock().unwrap();
        let Some(chain) = treemap_hit(&window, &state, x, y) else {
            return;
        };
        // Das erste Glied unterhalb der aktuellen Wurzel
        let Some(&next) = chain.get(state.treemap_root.len()) else {
            return;
        };
        let mut new_root = state.treemap_root.clone();
        new_root.push(next);
        let is_directory = state
            .tree
            .as_ref()
            .and_then(|tree| node_at(tree, &new_root))
            .is_some_and(|node| node.is_directory && !node.children.is_empty());
        if !is_directory {
            return;
        }
        state.treemap_root = new_root;
        show_treemap_root(&window, &state);
        drop(state);
        schedule_treemap_render(&gui_descend);
    });

    // "Hoch": eine Ebene zurück
    let gui_ascend = gui.clone();
    main_window.on_treemap_ascend(move || {
        let window = gui_ascend.window.unwrap();
        let mut state = gui_ascend.state.lock().unwrap();
        if state.treemap_root.pop().is_none() {
            return;
        }
        show_treemap_root(&window, &state);
        drop(state);
        schedule_treemap_render(&gui_ascend);
    });

    main_window.run()?;
    Ok(())
}

/// Dauer der beiden Scan-Phasen, getrennt gemessen
struct ScanTimings {
    /// MFT lesen und parsen
    scan: std::time::Duration,
    /// Verzeichnisbaum aufbauen
    tree: std::time::Duration,
}

/// Führt den MFT-Scan in einem Background-Thread durch
fn perform_scan_threaded(
    drive: &str,
    window_weak: slint::Weak<MainWindow>,
) -> Result<(TreeNode, ScanTimings), String> {
    // MFT-Reader erstellen
    let reader = MftReader::new(drive).map_err(|e| format!("{}", e))?;

    // Scan durchführen mit Progress-Updates via invoke_from_event_loop
    let scan_started = std::time::Instant::now();
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

    let scan = scan_started.elapsed();

    if entries.is_empty() {
        return Err("Keine Dateien gefunden. Admin-Rechte erforderlich!".to_string());
    }

    // Baum aufbauen
    let tree_started = std::time::Instant::now();
    let builder = TreeBuilder::new(entries);
    let mut tree = builder.build();
    tree.name = drive.to_string();
    let tree_time = tree_started.elapsed();

    Ok((
        tree,
        ScanTimings {
            scan,
            tree: tree_time,
        },
    ))
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

/// Baut die Liste neu auf und zeigt Markierung und Sortierung an
fn refresh_list(window: &MainWindow, state: &AppState) {
    let Some(tree) = state.tree.as_ref() else {
        return;
    };
    let entries = tree_to_entries(tree, &state.expanded, state.sort);
    let selected_index = state
        .selected
        .and_then(|id| {
            collect_visible_ids(tree, &state.expanded, state.sort)
                .iter()
                .position(|&visible| visible == id)
        })
        .map_or(-1, |index| index as i32);

    let entries_model: Rc<slint::VecModel<TreeEntry>> = Rc::new(slint::VecModel::from(entries));
    window.set_tree_entries(entries_model.into());
    window.set_selected_index(selected_index);
    window.set_sort_column(state.sort.column);
    window.set_sort_descending(state.sort.descending);
}

/// Scrollt die Liste so, dass die markierte Zeile in der Mitte liegt
fn scroll_to_selected(window: &MainWindow) {
    let index = window.get_selected_index();
    if index < 0 {
        return;
    }
    let visible = window.get_list_height();
    let total = window.get_tree_entries().row_count() as f32 * ROW_HEIGHT;
    let wanted = -(index as f32 * ROW_HEIGHT) + visible / 2.0 - ROW_HEIGHT / 2.0;
    let lowest = (visible - total).min(0.0);
    window.set_list_viewport_y(wanted.clamp(lowest, 0.0));
}

/// Zeigt den Pfad der Treemap-Wurzel im Kopf (leer für das Laufwerk)
fn show_treemap_root(window: &MainWindow, state: &AppState) {
    let text = if state.treemap_root.is_empty() {
        String::new()
    } else {
        state
            .tree
            .as_ref()
            .and_then(|tree| path_of(tree, &state.treemap_root))
            .unwrap_or_default()
    };
    window.set_treemap_root_text(SharedString::from(text));
}

/// Zeichnet die Treemap in der Größe des Anzeigebereichs, im Hintergrund
///
/// Das Bild entsteht in Gerätepixeln (Slint rechnet in logischen Pixeln,
/// bei 125 % Skalierung also 1,25 Gerätepixel je Einheit), damit es scharf
/// bleibt. Kommt vor dem Ende ein neuer Auftrag - Fenster wird weiter
/// gezogen -, verwirft der ältere sein Ergebnis.
fn schedule_treemap_render(gui: &Gui) {
    let Some(window) = gui.window.upgrade() else {
        return;
    };
    let scale = window.window().scale_factor();
    let width = (window.get_treemap_width() * scale).round() as u32;
    let height = (window.get_treemap_height() * scale).round() as u32;
    if width == 0 || height == 0 {
        return;
    }

    let my_generation = gui.render_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let gui = gui.clone();
    std::thread::spawn(move || {
        let map = {
            let state = gui.state.lock().unwrap();
            let Some(root) = state.treemap_root_node() else {
                return;
            };
            Treemap::render(root, width, height)
        };
        if gui.render_generation.load(Ordering::SeqCst) != my_generation {
            return; // überholt
        }

        let _ = slint::invoke_from_event_loop(move || {
            let Some(window) = gui.window.upgrade() else {
                return;
            };
            if gui.render_generation.load(Ordering::SeqCst) != my_generation {
                return;
            }
            let buffer =
                SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(map.pixels(), map.width(), map.height());
            window.set_treemap_image(Image::from_rgb8(buffer));
            gui.state.lock().unwrap().treemap = Some(map);
        });
    });
}

/// Der Knoten unter einem Mauspunkt der Treemap, als Kette von IDs ab der
/// Baumwurzel (die gezeigte Treemap-Wurzel inklusive)
fn treemap_hit(window: &MainWindow, state: &AppState, x: f32, y: f32) -> Option<Vec<u64>> {
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let scale = window.window().scale_factor();
    let hit = state
        .treemap
        .as_ref()?
        .hit((x * scale) as f64, (y * scale) as f64)?;
    let mut chain = state.treemap_root.clone();
    chain.extend_from_slice(hit.chain);
    Some(chain)
}

/// Text für die Hover-Zeile: Pfad und Größe, bei Ordnern auch die Dateizahl
fn describe(tree: &TreeNode, chain: &[u64]) -> Option<String> {
    let node = node_at(tree, chain)?;
    let path = path_of(tree, chain)?;
    Some(if node.is_directory {
        format!(
            "{} - {} ({} Dateien)",
            path,
            format_size(node.total_size),
            node.file_count
        )
    } else {
        format!("{} - {}", path, format_size(node.total_size))
    })
}

/// Folgt einer Kette von IDs von der Wurzel nach unten
fn node_at<'a>(root: &'a TreeNode, chain: &[u64]) -> Option<&'a TreeNode> {
    let mut node = root;
    for &id in chain {
        node = node.children.iter().find(|child| child.id == id)?;
    }
    Some(node)
}

/// Voller Pfad zu einer Kette von IDs, mit Backslashes wie im Explorer
fn path_of(root: &TreeNode, chain: &[u64]) -> Option<String> {
    let mut path = root.name.clone();
    let mut node = root;
    for &id in chain {
        node = node.children.iter().find(|child| child.id == id)?;
        path.push('\\');
        path.push_str(&node.name);
    }
    Some(path)
}

/// Die Kinder eines Knotens in der gewählten Sortierung
///
/// Der Baum selbst ist nach Größe absteigend sortiert; alles andere wird
/// nur für die sichtbaren Ordner umsortiert, nie für den ganzen Baum.
fn ordered_children(node: &TreeNode, sort: SortOrder) -> Vec<&TreeNode> {
    let mut children: Vec<&TreeNode> = node.children.iter().collect();
    if sort.column == SortColumn::Name {
        children.sort_by_cached_key(|child| child.name.to_lowercase());
        if sort.descending {
            children.reverse();
        }
    } else if !sort.descending {
        children.reverse();
    }
    children
}

/// Läuft über die sichtbaren Knoten in Listenreihenfolge: die Kinder der
/// Wurzel und darunter die aufgeklappten Ordner
fn visit_visible<'a, F>(root: &'a TreeNode, expanded: &HashSet<u64>, sort: SortOrder, visit: &mut F)
where
    F: FnMut(&'a TreeNode, i32, bool),
{
    fn walk<'a, F>(node: &'a TreeNode, expanded: &HashSet<u64>, sort: SortOrder, depth: i32, visit: &mut F)
    where
        F: FnMut(&'a TreeNode, i32, bool),
    {
        let is_expanded = node.is_directory && expanded.contains(&node.id);
        visit(node, depth, is_expanded);
        if is_expanded {
            for child in ordered_children(node, sort) {
                walk(child, expanded, sort, depth + 1, visit);
            }
        }
    }

    for child in ordered_children(root, sort) {
        walk(child, expanded, sort, 0, visit);
    }
}

/// Konvertiert den Baum in flache TreeEntry-Liste für die GUI
fn tree_to_entries(root: &TreeNode, expanded: &HashSet<u64>, sort: SortOrder) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    visit_visible(root, expanded, sort, &mut |node, depth, is_expanded| {
        entries.push(TreeEntry {
            name: SharedString::from(&node.name),
            size: SharedString::from(format_size(node.total_size)),
            size_bytes: node.total_size as f32,
            is_directory: node.is_directory,
            depth,
            expanded: is_expanded,
            has_children: node.is_directory && !node.children.is_empty(),
        });
    });
    entries
}

/// Sammelt die IDs der sichtbaren Knoten in der gleichen Reihenfolge wie
/// tree_to_entries, um einen Listen-Index einem Knoten zuzuordnen
fn collect_visible_ids(root: &TreeNode, expanded: &HashSet<u64>, sort: SortOrder) -> Vec<u64> {
    let mut ids = Vec::new();
    visit_visible(root, expanded, sort, &mut |node, _, _| ids.push(node.id));
    ids
}

/// CLI-Modus ohne GUI
fn run_cli(
    drive: Option<String>,
    dump_mft: Option<&std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
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
    if let Some(path) = dump_mft {
        println!("Scanne MFT, Dump nach {}...", path.display());
    } else {
        println!("Scanne MFT...");
    }
    let entries = reader.scan_with_dump(dump_mft, |progress, status| {
        print!("\r{:.0}% - {}", progress * 100.0, status);
        use std::io::Write;
        std::io::stdout().flush().ok();
    })?;
    let scan_time = started.elapsed();

    println!();
    println!();

    if entries.is_empty() {
        eprintln!("Keine Dateien gefunden. Admin-Rechte erforderlich!");
        return Ok(());
    }

    // Baum aufbauen
    println!("Baue Verzeichnisbaum...");
    let tree_started = std::time::Instant::now();
    let builder = TreeBuilder::new(entries);
    let mut tree = builder.build();
    tree.name = drive.to_string();
    let tree_time = tree_started.elapsed();

    // Debug: Zeige erste Kinder des Root
    println!();
    println!("Root-Kinder (erste 10):");
    println!("─────────────────────────────────────────────────");
    for (i, child) in tree.children.iter().take(10).enumerate() {
        println!(
            "{:2}. [{}] {:<40} {}",
            i + 1,
            if child.is_directory { "DIR " } else { "FILE" },
            child.name,
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
            item.path
        );
    }

    println!();
    println!(
        "Gesamt: {} Dateien, {} Ordner",
        tree.file_count, tree.dir_count
    );
    println!("Gesamtgröße: {}", format_size(tree.total_size));
    println!(
        "Fertig in {} (Scan {}, Baum {})",
        format_duration(started.elapsed()),
        format_duration(scan_time),
        format_duration(tree_time)
    );

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
