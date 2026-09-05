//! TreeBuilder - Konstruiert den Verzeichnisbaum aus MFT-Einträgen
//!
//! # Algorithmus
//!
//! 1. Alle MFT-Einträge sind in einer HashMap (mft_reference -> FileEntry)
//! 2. Wir starten beim Root-Verzeichnis (MFT-Referenz 5)
//! 3. Für jeden Ordner finden wir alle Kinder (Einträge mit parent_reference == unsere Referenz)
//! 4. Rekursiv bauen wir so den Baum auf
//! 5. Zum Schluss wird der fertige Baum einmal sortiert
//!
//! # Performance-Tricks
//!
//! Statt für jeden Ordner die HashMap zu durchsuchen, bauen wir zuerst
//! einen Index: parent_reference -> Vec<mft_reference>
//! Das macht die Suche O(1) statt O(n).
//!
//! Sortiert wird genau einmal, von der Wurzel aus: würde jeder Ordner beim
//! Aufbau seinen Teilbaum sortieren, wäre ein Ordner in Tiefe 12 zwölfmal
//! sortiert. Und kein Knoten trägt seinen vollen Pfad - bei Millionen
//! Einträgen wären das hunderte MB an Strings, die fast nie gebraucht werden.

use super::node::TreeNode;
use crate::mft::FileEntry;
use std::collections::HashMap;

/// MFT-Referenz des Root-Verzeichnisses
const ROOT_REFERENCE: u64 = 5;

/// Der TreeBuilder konstruiert den Baum aus MFT-Daten
pub struct TreeBuilder {
    /// Alle Dateien, indexiert nach MFT-Referenz
    entries: HashMap<u64, FileEntry>,

    /// Index: Parent-Referenz -> Liste der Kind-Referenzen
    children_index: HashMap<u64, Vec<u64>>,
}

/// Ein Treffer von [`TreeBuilder::top_n`] - mit vollem Pfad, den der Baum
/// selbst nicht speichert
#[derive(Debug, Clone)]
pub struct TopEntry {
    /// Voller Pfad, z.B. `C:\Windows\System32`
    pub path: String,
    /// Name der Datei/des Ordners
    pub name: String,
    /// Gesamtgröße inkl. Kinder
    pub total_size: u64,
    /// Ist dies ein Ordner?
    pub is_directory: bool,
}

impl TreeBuilder {
    /// Erstellt einen neuen TreeBuilder aus MFT-Einträgen
    pub fn new(entries: HashMap<u64, FileEntry>) -> Self {
        // Kinder-Index aufbauen
        let mut children_index: HashMap<u64, Vec<u64>> = HashMap::new();

        for (mft_ref, entry) in &entries {
            children_index
                .entry(entry.parent_reference)
                .or_default()
                .push(*mft_ref);
        }

        Self {
            entries,
            children_index,
        }
    }

    /// Baut den Baum ab dem Root-Verzeichnis auf, sortiert nach Größe
    pub fn build(&self) -> TreeNode {
        let mut root = self.build_subtree(ROOT_REFERENCE);
        root.sort_by_size();
        root
    }

    /// Baut einen Teilbaum rekursiv auf (unsortiert)
    fn build_subtree(&self, mft_reference: u64) -> TreeNode {
        // Eigenen Eintrag holen
        let Some(entry) = self.entries.get(&mft_reference) else {
            // Fallback für Root wenn nicht in HashMap
            let mut node = TreeNode::new_directory(String::new());
            node.id = mft_reference;
            return node;
        };

        if !entry.is_directory {
            let mut node = TreeNode::new_file(entry.name.clone(), entry.size);
            node.id = mft_reference;
            return node;
        }

        let mut node = TreeNode::new_directory(entry.name.clone());
        node.id = mft_reference;

        // Alle Kinder dieses Ordners holen
        if let Some(child_refs) = self.children_index.get(&mft_reference) {
            node.children.reserve_exact(child_refs.len());
            for &child_ref in child_refs {
                // Vermeide Endlosrekursion: Root zeigt auf sich selbst
                if child_ref == mft_reference {
                    continue;
                }
                node.add_child(self.build_subtree(child_ref));
            }
        }

        node
    }

    /// Gibt die N größten Ordner/Dateien unterhalb der Wurzel zurück,
    /// größte zuerst, mit vollem Pfad
    ///
    /// Ein Teilbaum kann nie größer sein als sein Ordner; Ordner, die es
    /// nicht in die Top N schaffen, werden deshalb samt Inhalt übersprungen.
    /// Pfad-Strings entstehen nur für Kandidaten, nicht für jeden Knoten.
    pub fn top_n(root: &TreeNode, n: usize) -> Vec<TopEntry> {
        let mut best: Vec<TopEntry> = Vec::with_capacity(n + 1);
        let mut path: Vec<&str> = vec![&root.name];
        for child in &root.children {
            Self::collect_top(child, &mut path, &mut best, n);
        }
        best
    }

    /// Rekursive Hilfsfunktion für top_n: `path` ist der Weg bis zum Parent
    fn collect_top<'a>(
        node: &'a TreeNode,
        path: &mut Vec<&'a str>,
        best: &mut Vec<TopEntry>,
        n: usize,
    ) {
        if n == 0 {
            return;
        }
        let threshold = if best.len() < n {
            None
        } else {
            best.last().map(|entry| entry.total_size)
        };
        if threshold.is_some_and(|min| node.total_size <= min) {
            return; // weder dieser Knoten noch seine Kinder kommen rein
        }

        let full_path = {
            let mut joined = path.join("\\");
            joined.push('\\');
            joined.push_str(&node.name);
            joined
        };
        let position = best.partition_point(|entry| entry.total_size >= node.total_size);
        best.insert(
            position,
            TopEntry {
                path: full_path,
                name: node.name.clone(),
                total_size: node.total_size,
                is_directory: node.is_directory,
            },
        );
        if best.len() > n {
            best.pop();
        }

        path.push(&node.name);
        for child in &node.children {
            Self::collect_top(child, path, best, n);
        }
        path.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tree_building() {
        let mut entries = HashMap::new();

        // Root (MFT 5)
        entries.insert(5, FileEntry::new(5, 5, String::new(), 0, true));

        // Ordner "Users" (MFT 100)
        entries.insert(100, FileEntry::new(100, 5, "Users".to_string(), 0, true));

        // Datei "test.txt" in Users (MFT 101)
        entries.insert(101, FileEntry::new(101, 100, "test.txt".to_string(), 1024, false));

        let builder = TreeBuilder::new(entries);
        let tree = builder.build();

        assert_eq!(tree.id, 5);
        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].name, "Users");
        assert_eq!(tree.children[0].id, 100);
        assert_eq!(tree.children[0].total_size, 1024);
        assert_eq!(tree.children[0].children[0].id, 101);
    }

    #[test]
    fn build_sorts_every_level_by_size() {
        let mut entries = HashMap::new();
        entries.insert(5, FileEntry::new(5, 5, String::new(), 0, true));
        entries.insert(10, FileEntry::new(10, 5, "klein".to_string(), 0, true));
        entries.insert(11, FileEntry::new(11, 5, "gross".to_string(), 0, true));
        entries.insert(20, FileEntry::new(20, 10, "a".to_string(), 10, false));
        entries.insert(21, FileEntry::new(21, 11, "b".to_string(), 1, false));
        entries.insert(22, FileEntry::new(22, 11, "c".to_string(), 100, false));

        let tree = TreeBuilder::new(entries).build();
        let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["gross", "klein"]);
        let inner: Vec<&str> = tree.children[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(inner, vec!["c", "b"]);
    }

    #[test]
    fn top_n_returns_largest_with_paths() {
        let mut root = TreeNode::new_directory("C:".to_string());
        let mut windows = TreeNode::new_directory("Windows".to_string());
        windows.add_child(TreeNode::new_file("explorer.exe".to_string(), 500));
        let mut system32 = TreeNode::new_directory("System32".to_string());
        system32.add_child(TreeNode::new_file("ntoskrnl.exe".to_string(), 9000));
        windows.add_child(system32);
        root.add_child(windows);
        root.add_child(TreeNode::new_file("pagefile.sys".to_string(), 4000));

        let top = TreeBuilder::top_n(&root, 3);
        let paths: Vec<&str> = top.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "C:\\Windows",
                "C:\\Windows\\System32",
                "C:\\Windows\\System32\\ntoskrnl.exe"
            ]
        );
        assert_eq!(top[0].total_size, 9500);
        assert!(top[0].is_directory);
        assert!(TreeBuilder::top_n(&root, 0).is_empty());
    }
}
