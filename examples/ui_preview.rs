//! UI-Vorschau mit Beispieldaten
//!
//! Zeigt das Hauptfenster mit einem festen Beispielbaum, ohne MFT-Scan und
//! ohne Admin-Rechte: Examples bekommen im Gegensatz zum Binary kein
//! Manifest (siehe build.rs). Praktisch, um am Layout zu arbeiten:
//!
//! ```bash
//! cargo run --example ui_preview
//! ```
//!
//! Die Treemap wird aus demselben Beispielbaum gezeichnet; Hover und Klick
//! zeigen den Pfad, wie in der App.

use rustree::tree::{format_size, TreeNode};
use rustree::treemap::Treemap;
use slint::{Image, Model, Rgb8Pixel, SharedPixelBuffer, SharedString, VecModel};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

slint::include_modules!();

/// Baut einen Ordner mit Kindern; IDs werden durchnummeriert (Cell, damit
/// verschachtelte Aufrufe denselben Zähler teilen können)
fn dir(name: &str, next_id: &Cell<u64>, children: Vec<TreeNode>) -> TreeNode {
    let mut node = TreeNode::new_directory(name.to_string());
    node.id = next_id.replace(next_id.get() + 1);
    for child in children {
        node.add_child(child);
    }
    node
}

fn file(name: &str, size: u64, next_id: &Cell<u64>) -> TreeNode {
    let mut node = TreeNode::new_file(name.to_string(), size);
    node.id = next_id.replace(next_id.get() + 1);
    node
}

/// Ein kleines Laufwerk mit typischen Ausreißern
fn sample_tree() -> TreeNode {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    let id = &Cell::new(100u64);
    let mut root = dir(
        "C:",
        id,
        vec![
            dir(
                "Users",
                id,
                vec![
                    dir(
                        "kevin",
                        id,
                        vec![
                            dir(
                                "Videos",
                                id,
                                vec![
                                    file("urlaub-2025.mp4", 38 * GB, id),
                                    file("konzert.mkv", 21 * GB, id),
                                    file("clip.mov", 3 * GB, id),
                                ],
                            ),
                            dir(
                                ".android",
                                id,
                                vec![file("pixel_8.img", 26 * GB, id), file("emulator.qcow2", 12 * GB, id)],
                            ),
                            dir(
                                "AppData",
                                id,
                                (0..40).map(|i| file(&format!("cache-{i}.bin"), 200 * MB, id)).collect(),
                            ),
                            file("Dateiname mit Umlauten äöü ß.txt", 1024, id),
                        ],
                    ),
                    dir("kmwork", id, vec![file("mailbox.ost", 9 * GB, id), file("notes.one", 400 * MB, id)]),
                ],
            ),
            dir(
                "Windows",
                id,
                vec![
                    dir("System32", id, (0..60).map(|i| file(&format!("lib{i}.dll"), 120 * MB, id)).collect()),
                    dir("WinSxS", id, (0..80).map(|i| file(&format!("comp{i}.manifest"), 70 * MB, id)).collect()),
                    file("explorer.exe", 5 * MB, id),
                ],
            ),
            dir(
                "ProgramData",
                id,
                vec![file("Windows 11 dev environment.vhdx", 118 * GB, id), file("setup.log", 12 * MB, id)],
            ),
            dir("Program Files (x86)", id, (0..25).map(|i| file(&format!("app{i}.exe"), 400 * MB, id)).collect()),
            file("pagefile.sys", 4 * GB, id),
            file("swapfile.sys", 16 * MB, id),
            file(
                "Ein sehr langer Dateiname, der am rechten Rand abgeschnitten werden muss, damit die Größe sichtbar bleibt.log",
                12 * 1024,
                id,
            ),
        ],
    );
    root.sort_by_size();
    root
}

/// Sichtbare Zeilen nach aktuellem Auf-/Zu-Zustand, eine kleine Fassung
/// von `tree_to_entries` in main.rs; dazu die IDs in derselben Reihenfolge,
/// um einen Listenindex aus einem Callback einem Knoten zuzuordnen
fn visible_rows(root: &TreeNode, expanded: &HashSet<u64>) -> (Vec<TreeEntry>, Vec<u64>) {
    fn walk(node: &TreeNode, depth: i32, expanded: &HashSet<u64>, entries: &mut Vec<TreeEntry>, ids: &mut Vec<u64>) {
        for child in &node.children {
            let is_expanded = child.is_directory && expanded.contains(&child.id);
            entries.push(TreeEntry {
                name: SharedString::from(&child.name),
                size: SharedString::from(format_size(child.total_size)),
                size_bytes: child.total_size as f32,
                is_directory: child.is_directory,
                depth,
                expanded: is_expanded,
                has_children: child.is_directory && !child.children.is_empty(),
            });
            ids.push(child.id);
            if is_expanded {
                walk(child, depth + 1, expanded, entries, ids);
            }
        }
    }
    let mut entries = Vec::new();
    let mut ids = Vec::new();
    walk(root, 0, expanded, &mut entries, &mut ids);
    (entries, ids)
}

/// Baut die Liste neu auf - wie `refresh_list` in main.rs mit einem neuen
/// VecModel, damit die Vorschau denselben Modellwechsel macht wie die App
/// (das Kontextmenü muss ihn überstehen, siehe context_index in main.slint)
fn refresh(window: &MainWindow, tree: &TreeNode, expanded: &HashSet<u64>, ids_out: &RefCell<Vec<u64>>, selected_id: Option<u64>) {
    let (entries, ids) = visible_rows(tree, expanded);
    let selected_index = selected_id
        .and_then(|id| ids.iter().position(|&x| x == id))
        .map_or(-1, |i| i as i32);
    *ids_out.borrow_mut() = ids;
    window.set_tree_entries(Rc::new(VecModel::from(entries)).into());
    window.set_selected_index(selected_index);
}

/// Name der Zeile an `index`, aus dem Modell, das die GUI gerade zeigt
fn describe_row(window: &MainWindow, index: i32) -> String {
    if index < 0 {
        return "<keine Zeile>".to_string();
    }
    window
        .get_tree_entries()
        .row_data(index as usize)
        .map(|e| e.name.to_string())
        .unwrap_or_else(|| format!("<ungültiger Index {index}>"))
}

/// Statustext für eine Kontextmenü-Aktion, zusätzlich auf stderr - so lässt
/// sich das Menü ohne Scan prüfen (die Vorschau tut sonst nichts damit)
fn show_action(window: &MainWindow, action: &str, index: i32) {
    let text = format!("Kontextmenü: {action}, Zeile {index}: {}", describe_row(window, index));
    eprintln!("{text}");
    window.set_status_text(SharedString::from(text));
}

/// Pfad und Größe zu einer ID-Kette, wie die Hover-Zeile der App
fn describe(root: &TreeNode, chain: &[u64]) -> String {
    let mut path = root.name.clone();
    let mut node = root;
    for &id in chain {
        let Some(child) = node.children.iter().find(|c| c.id == id) else {
            return String::new();
        };
        path.push('\\');
        path.push_str(&child.name);
        node = child;
    }
    format!("{} - {}", path, format_size(node.total_size))
}

fn main() -> Result<(), slint::PlatformError> {
    let window = MainWindow::new()?;
    let tree = Rc::new(sample_tree());
    let treemap: Rc<RefCell<Option<Treemap>>> = Rc::new(RefCell::new(None));

    // Baumzustand wie AppState in main.rs: aufgeklappte Ordner per ID,
    // dazu die IDs der sichtbaren Zeilen für die Callbacks
    let expanded: Rc<RefCell<HashSet<u64>>> = Rc::new(RefCell::new({
        let mut set = HashSet::new();
        if let Some(first) = tree.children.first() {
            set.insert(first.id); // erstes Kind schon aufgeklappt, wie bisher
        }
        set
    }));
    let ids: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));
    let selected_id: Rc<Cell<Option<u64>>> = Rc::new(Cell::new(
        // Markiert wie bisher die VHDX-Datei, das erste Kind des ersten
        // (größten) Ordners - siehe Treemap-Markierung weiter unten
        tree.children.first().and_then(|c| c.children.first()).map(|c| c.id),
    ));

    refresh(&window, &tree, &expanded.borrow(), &ids, selected_id.get());
    window.set_status_text(SharedString::from("Vorschau - Beispieldaten"));
    window.set_scan_target(SharedString::from("C:"));

    // Linksklick auf eine Zeile: Ordner auf-/zuklappen, wie in main.rs
    {
        let window_weak = window.as_weak();
        let tree = tree.clone();
        let expanded = expanded.clone();
        let ids = ids.clone();
        let selected_id = selected_id.clone();
        window.on_toggle_entry(move |index| {
            let window = window_weak.unwrap();
            let id = ids.borrow().get(index as usize).copied();
            if let Some(id) = id {
                let mut set = expanded.borrow_mut();
                if !set.remove(&id) {
                    set.insert(id);
                }
                drop(set);
                selected_id.set(Some(id));
            }
            let expanded_snapshot = expanded.borrow().clone();
            refresh(&window, &tree, &expanded_snapshot, &ids, selected_id.get());
            let text = format!("toggle_entry, Zeile {index}: {}", describe_row(&window, index));
            eprintln!("{text}");
            window.set_status_text(SharedString::from(text));
        });
    }

    // Rechtsklick auf eine Zeile: nur markieren, wie in main.rs
    {
        let window_weak = window.as_weak();
        let tree = tree.clone();
        let expanded = expanded.clone();
        let ids = ids.clone();
        let selected_id = selected_id.clone();
        window.on_select_entry(move |index| {
            let window = window_weak.unwrap();
            let id = ids.borrow().get(index as usize).copied();
            selected_id.set(id);
            let expanded_snapshot = expanded.borrow().clone();
            refresh(&window, &tree, &expanded_snapshot, &ids, selected_id.get());
            let text = format!("select_entry, Zeile {index}: {}", describe_row(&window, index));
            eprintln!("{text}");
            window.set_status_text(SharedString::from(text));
            window.window().request_redraw();
        });
    }

    // Kontextmenü-Aktionen: nur melden, welche Zeile sie getroffen haben
    {
        let window_weak = window.as_weak();
        window.on_open_in_explorer(move |index| show_action(&window_weak.unwrap(), "Im Explorer öffnen", index));
    }
    {
        let window_weak = window.as_weak();
        window.on_copy_path(move |index| show_action(&window_weak.unwrap(), "Pfad kopieren", index));
    }
    {
        let window_weak = window.as_weak();
        window.on_zoom_treemap(move |index| show_action(&window_weak.unwrap(), "In Treemap zeigen", index));
    }
    {
        let window_weak = window.as_weak();
        window.on_delete_entry(move |index| show_action(&window_weak.unwrap(), "Löschen (Papierkorb)", index));
    }

    // Laufwerke für die Startansicht; RUSTREE_PREVIEW_START=1 zeigt sie
    // statt des Scan-Ergebnisses
    let drive = |letter: &str, label: &str, fs: &str, total: &str, free: &str, used: f32| DriveInfo {
        letter: SharedString::from(letter),
        label: SharedString::from(label),
        file_system: SharedString::from(fs),
        total: SharedString::from(total),
        free: SharedString::from(free),
        used,
        used_text: SharedString::from(format!("{:.0} %", used * 100.0)),
        scannable: fs == "NTFS",
    };
    window.set_drives(
        Rc::new(VecModel::from(vec![
            drive("C:", "M2_1", "NTFS", "3.72 TB", "2.41 TB", 0.35),
            drive("D:", "SSD2", "NTFS", "3.64 TB", "595.70 GB", 0.84),
            drive("E:", "SSD3", "exFAT", "931.51 GB", "130.80 GB", 0.86),
            drive("F:", "M2_2", "NTFS", "1.82 TB", "151.54 GB", 0.92),
        ]))
        .into(),
    );
    window.set_available_drives(
        Rc::new(VecModel::from(vec![
            SharedString::from("C:"),
            SharedString::from("D:"),
            SharedString::from("E:"),
            SharedString::from("F:"),
        ]))
        .into(),
    );
    window.set_has_tree(std::env::var_os("RUSTREE_PREVIEW_START").is_none());

    // Treemap in der Größe des Bereichs zeichnen, auch nach Größenänderung
    let window_weak = window.as_weak();
    let tree_render = tree.clone();
    let treemap_render = treemap.clone();
    let render = move || {
        let window = window_weak.unwrap();
        let scale = window.window().scale_factor();
        let width = (window.get_treemap_width() * scale).round() as u32;
        let height = (window.get_treemap_height() * scale).round() as u32;
        if width == 0 || height == 0 {
            return;
        }
        let map = Treemap::render(&tree_render, width, height);
        let buffer = SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(map.pixels(), width, height);
        window.set_treemap_image(Image::from_rgb8(buffer));

        // Markierung wie nach einem Klick: die zweite Zeile der Liste ist
        // die VHDX-Datei im ersten Ordner
        let first = &tree_render.children[0];
        if let Some(bounds) = map.bounds_of(&[first.id, first.children[0].id]) {
            window.set_highlight_x(bounds.x0 as f32 / scale);
            window.set_highlight_y(bounds.y0 as f32 / scale);
            window.set_highlight_width((bounds.x1 - bounds.x0) as f32 / scale);
            window.set_highlight_height((bounds.y1 - bounds.y0) as f32 / scale);
            window.set_highlight_visible(true);
        }
        *treemap_render.borrow_mut() = Some(map);
    };
    window.on_treemap_resized(render);

    let window_weak = window.as_weak();
    let tree_hover = tree.clone();
    let treemap_hover = treemap.clone();
    window.on_treemap_hover(move |x, y| {
        let window = window_weak.unwrap();
        let scale = window.window().scale_factor();
        let text = treemap_hover
            .borrow()
            .as_ref()
            .and_then(|map| map.hit((x * scale) as f64, (y * scale) as f64).map(|hit| hit.chain.to_vec()))
            .map(|chain| describe(&tree_hover, &chain))
            .unwrap_or_default();
        window.set_treemap_hover_text(SharedString::from(text));
    });

    window.run()
}
