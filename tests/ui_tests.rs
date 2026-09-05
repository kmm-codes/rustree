//! UI E2E Tests für rustree
//!
//! Diese Tests prüfen automatisch verschiedene UI-Abläufe.
//!
//! # Ausführen
//! ```bash
//! cargo test --test ui_tests
//! ```
//!
//! Hinweis: Diese Tests fokussieren sich auf die Logik, nicht auf visuelles Rendering.

use slint::{Model, SharedString, VecModel};
use std::rc::Rc;

// Importiere die generierten Slint-Typen
slint::include_modules!();

/// Test: TreeEntry-Struktur kann erstellt werden
#[test]
fn test_tree_entry_creation() {
    let entry = TreeEntry {
        name: SharedString::from("TestFolder"),
        size: SharedString::from("1.5 GB"),
        size_bytes: 1.5e9,
        is_directory: true,
        depth: 0,
        expanded: false,
        has_children: true,
    };

    assert_eq!(entry.name, "TestFolder");
    assert!(entry.is_directory);
    assert_eq!(entry.depth, 0);
}

/// Test: VecModel kann mit TreeEntries befüllt werden
#[test]
fn test_tree_entry_model() {
    let entries = vec![
        TreeEntry {
            name: SharedString::from("Windows"),
            size: SharedString::from("25 GB"),
            size_bytes: 25e9,
            is_directory: true,
            depth: 0,
            expanded: false,
            has_children: true,
        },
        TreeEntry {
            name: SharedString::from("Users"),
            size: SharedString::from("45 GB"),
            size_bytes: 45e9,
            is_directory: true,
            depth: 0,
            expanded: true,
            has_children: true,
        },
    ];

    let model: Rc<VecModel<TreeEntry>> = Rc::new(VecModel::from(entries));
    assert_eq!(model.row_count(), 2);
}

/// Test: SharedString-Konvertierung funktioniert
#[test]
fn test_shared_string_conversion() {
    let drives = vec![
        SharedString::from("C:"),
        SharedString::from("D:"),
        SharedString::from("E:"),
    ];

    let model: Rc<VecModel<SharedString>> = Rc::new(VecModel::from(drives));
    assert_eq!(model.row_count(), 3);
}

// ============================================================================
// MFT Module Unit Tests
// ============================================================================

#[test]
fn test_format_size() {
    use rustree::tree::format_size;

    assert_eq!(format_size(0), "0 B");
    assert_eq!(format_size(500), "500 B");
    assert_eq!(format_size(1024), "1.00 KB");
    assert_eq!(format_size(1536), "1.50 KB");
    assert_eq!(format_size(1024 * 1024), "1.00 MB");
    assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GB");
    assert_eq!(format_size(1024u64 * 1024 * 1024 * 1024), "1.00 TB");
}

#[test]
fn test_tree_node_creation() {
    use rustree::tree::TreeNode;

    let file = TreeNode::new_file("test.txt".to_string(), 1024);
    assert_eq!(file.name, "test.txt");
    assert_eq!(file.total_size, 1024);
    assert!(!file.is_directory);

    let dir = TreeNode::new_directory("Documents".to_string());
    assert_eq!(dir.name, "Documents");
    assert!(dir.is_directory);
    assert_eq!(dir.total_size, 0);
}

#[test]
fn test_tree_node_add_child() {
    use rustree::tree::TreeNode;

    let mut dir = TreeNode::new_directory("Documents".to_string());
    let file1 = TreeNode::new_file("file1.txt".to_string(), 1000);
    let file2 = TreeNode::new_file("file2.txt".to_string(), 2000);

    dir.add_child(file1);
    dir.add_child(file2);

    assert_eq!(dir.children.len(), 2);
    assert_eq!(dir.total_size, 3000);
    assert_eq!(dir.file_count, 2);
}

#[test]
fn test_tree_builder() {
    use rustree::mft::FileEntry;
    use rustree::tree::TreeBuilder;
    use std::collections::HashMap;

    let mut entries = HashMap::new();

    // Root (MFT 5)
    entries.insert(
        5,
        FileEntry {
            mft_reference: 5,
            parent_reference: 5,
            name: String::new(),
            size: 0,
            is_directory: true,
            is_hidden: false,
            is_system: false,
        },
    );

    // Ordner "Users" (MFT 100)
    entries.insert(
        100,
        FileEntry {
            mft_reference: 100,
            parent_reference: 5,
            name: "Users".to_string(),
            size: 0,
            is_directory: true,
            is_hidden: false,
            is_system: false,
        },
    );

    // Datei "test.txt" in Users (MFT 101)
    entries.insert(
        101,
        FileEntry {
            mft_reference: 101,
            parent_reference: 100,
            name: "test.txt".to_string(),
            size: 1024,
            is_directory: false,
            is_hidden: false,
            is_system: false,
        },
    );

    let builder = TreeBuilder::new(entries);
    let tree = builder.build();

    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].name, "Users");
    assert_eq!(tree.children[0].total_size, 1024);
    assert_eq!(tree.children[0].children.len(), 1);
    assert_eq!(tree.children[0].children[0].name, "test.txt");
}

#[test]
fn test_tree_sorting() {
    use rustree::tree::TreeNode;

    let mut dir = TreeNode::new_directory("Root".to_string());
    dir.add_child(TreeNode::new_file("small.txt".to_string(), 100));
    dir.add_child(TreeNode::new_file("large.txt".to_string(), 10000));
    dir.add_child(TreeNode::new_file("medium.txt".to_string(), 1000));

    dir.sort_by_size();

    // Nach Größe sortiert: large, medium, small
    assert_eq!(dir.children[0].name, "large.txt");
    assert_eq!(dir.children[1].name, "medium.txt");
    assert_eq!(dir.children[2].name, "small.txt");
}

#[test]
fn test_top_n() {
    use rustree::tree::{TreeBuilder, TreeNode};

    let mut root = TreeNode::new_directory("Root".to_string());

    let mut docs = TreeNode::new_directory("Documents".to_string());
    docs.add_child(TreeNode::new_file("big.pdf".to_string(), 50000));
    docs.add_child(TreeNode::new_file("small.txt".to_string(), 100));

    let mut pics = TreeNode::new_directory("Pictures".to_string());
    pics.add_child(TreeNode::new_file("huge.raw".to_string(), 100000));

    root.add_child(docs);
    root.add_child(pics);

    let top = TreeBuilder::top_n(&root, 3);

    // Top 3 nach Größe
    assert!(top.len() <= 3);
    assert!(top[0].total_size >= top[1].total_size);
}
