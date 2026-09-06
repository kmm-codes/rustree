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
use slint::{Image, Rgb8Pixel, SharedPixelBuffer, SharedString, VecModel};
use std::cell::{Cell, RefCell};
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

/// Erste Ebene plus die Kinder des größten Ordners, wie nach einem Klick
fn entries_for(root: &TreeNode) -> Vec<TreeEntry> {
    let mut entries = Vec::new();
    for (i, child) in root.children.iter().enumerate() {
        let expanded = i == 0 && child.is_directory;
        entries.push(entry(child, 0, expanded));
        if expanded {
            for grandchild in &child.children {
                entries.push(entry(grandchild, 1, false));
            }
        }
    }
    entries
}

fn entry(node: &TreeNode, depth: i32, expanded: bool) -> TreeEntry {
    TreeEntry {
        name: SharedString::from(&node.name),
        size: SharedString::from(format_size(node.total_size)),
        size_bytes: node.total_size as f32,
        is_directory: node.is_directory,
        depth,
        expanded,
        has_children: !node.children.is_empty(),
    }
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

    window.set_tree_entries(Rc::new(VecModel::from(entries_for(&tree))).into());
    window.set_selected_index(2);
    window.set_status_text(SharedString::from("Vorschau - Beispieldaten"));

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
