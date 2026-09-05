//! TreeNode - Ein Knoten im Verzeichnisbaum
//!
//! Jeder Knoten repräsentiert entweder eine Datei oder einen Ordner.
//! Ordner haben Kinder (children), die rekursiv die Baumstruktur bilden.
//!
//! Bewusst schlank: bei Millionen Knoten zählt jedes Feld. Der volle Pfad
//! wird nicht gespeichert, sondern bei Bedarf aus dem Weg durch den Baum
//! gebildet (siehe [`crate::tree::TreeBuilder::top_n`]).

use rayon::prelude::*;
use std::cmp::Ordering;

/// Ein Knoten im Verzeichnisbaum
#[derive(Debug, Clone)]
pub struct TreeNode {
    /// MFT-Referenz des Eintrags - eindeutig pro Laufwerk, 0 wenn unbekannt.
    /// Die GUI merkt sich darüber, welche Ordner aufgeklappt sind.
    pub id: u64,

    /// Name der Datei/des Ordners
    pub name: String,

    /// Eigene Größe in Bytes (für Dateien)
    pub own_size: u64,

    /// Gesamtgröße inkl. aller Kinder (für Ordner)
    pub total_size: u64,

    /// Ist dies ein Ordner?
    pub is_directory: bool,

    /// Kindknoten (nur bei Ordnern)
    pub children: Vec<TreeNode>,

    /// Anzahl der Dateien in diesem Teilbaum
    pub file_count: u64,

    /// Anzahl der Ordner in diesem Teilbaum
    pub dir_count: u64,
}

impl TreeNode {
    /// Erstellt einen neuen Datei-Knoten
    pub fn new_file(name: String, size: u64) -> Self {
        Self {
            id: 0,
            name,
            own_size: size,
            total_size: size,
            is_directory: false,
            children: Vec::new(),
            file_count: 1,
            dir_count: 0,
        }
    }

    /// Erstellt einen neuen Ordner-Knoten
    pub fn new_directory(name: String) -> Self {
        Self {
            id: 0,
            name,
            own_size: 0,
            total_size: 0,
            is_directory: true,
            children: Vec::new(),
            file_count: 0,
            dir_count: 1,
        }
    }

    /// Fügt ein Kind hinzu und aktualisiert die Größe
    pub fn add_child(&mut self, child: TreeNode) {
        self.total_size += child.total_size;
        self.file_count += child.file_count;
        self.dir_count += child.dir_count;
        self.children.push(child);
    }

    /// Sortiert die Kinder nach Größe (größte zuerst), rekursiv für den
    /// ganzen Teilbaum. Einmal an der Wurzel aufrufen, nicht pro Ebene:
    /// sonst wird jeder Ordner so oft sortiert, wie er tief liegt.
    pub fn sort_by_size(&mut self) {
        self.sort_by_size_at(0);
    }

    /// Wie [`TreeNode::sort_by_size`]; die obersten Ebenen parallel
    fn sort_by_size_at(&mut self, depth: usize) {
        self.children
            .sort_unstable_by(|a, b| b.total_size.cmp(&a.total_size));

        if depth < super::builder::PARALLEL_DEPTH {
            self.children
                .par_iter_mut()
                .filter(|child| child.is_directory)
                .for_each(|child| child.sort_by_size_at(depth + 1));
        } else {
            for child in self.children.iter_mut().filter(|child| child.is_directory) {
                child.sort_by_size_at(depth + 1);
            }
        }
    }

    /// Gibt die Größe als human-readable String zurück
    pub fn size_string(&self) -> String {
        format_size(self.total_size)
    }

    /// Berechnet den Prozentanteil an der Gesamtgröße
    pub fn percentage_of(&self, total: u64) -> f64 {
        if total == 0 {
            0.0
        } else {
            (self.total_size as f64 / total as f64) * 100.0
        }
    }

    /// Traversiert den Baum mit einer Callback-Funktion
    pub fn traverse<F>(&self, depth: usize, callback: &mut F)
    where
        F: FnMut(&TreeNode, usize),
    {
        callback(self, depth);
        for child in &self.children {
            child.traverse(depth + 1, callback);
        }
    }

    /// Gibt die Anzahl aller Knoten im Teilbaum zurück
    pub fn node_count(&self) -> u64 {
        1 + self.children.iter().map(|c| c.node_count()).sum::<u64>()
    }
}

/// Formatiert eine Größe in Bytes als human-readable String
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

impl Ord for TreeNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Größere Dateien zuerst
        other.total_size.cmp(&self.total_size)
    }
}

impl PartialOrd for TreeNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for TreeNode {
    fn eq(&self, other: &Self) -> bool {
        self.total_size == other.total_size && self.name == other.name
    }
}

impl Eq for TreeNode {}
