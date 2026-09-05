//! TreeBuilder - Konstruiert den Verzeichnisbaum aus MFT-Einträgen
//!
//! # Algorithmus
//!
//! 1. Alle MFT-Einträge sind in einer HashMap (mft_reference -> FileEntry)
//! 2. Wir starten beim Root-Verzeichnis (MFT-Referenz 5)
//! 3. Für jeden Ordner finden wir alle Kinder (Einträge mit parent_reference == unsere Referenz)
//! 4. Rekursiv bauen wir so den Baum auf
//!
//! # Performance-Trick
//!
//! Statt für jeden Ordner die HashMap zu durchsuchen, bauen wir zuerst
//! einen Index: parent_reference -> Vec<mft_reference>
//! Das macht die Suche O(1) statt O(n).

use super::node::TreeNode;
use crate::mft::FileEntry;
use std::collections::HashMap;

/// Der TreeBuilder konstruiert den Baum aus MFT-Daten
pub struct TreeBuilder {
    /// Alle Dateien, indexiert nach MFT-Referenz
    entries: HashMap<u64, FileEntry>,

    /// Index: Parent-Referenz -> Liste der Kind-Referenzen
    children_index: HashMap<u64, Vec<u64>>,
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

    /// Baut den Baum ab dem Root-Verzeichnis auf
    pub fn build(&self) -> TreeNode {
        // Root hat MFT-Referenz 5
        self.build_subtree(5, "")
    }

    /// Baut einen Teilbaum rekursiv auf
    fn build_subtree(&self, mft_reference: u64, parent_path: &str) -> TreeNode {
        // Eigenen Eintrag holen
        let entry = match self.entries.get(&mft_reference) {
            Some(e) => e,
            None => {
                // Fallback für Root wenn nicht in HashMap
                return TreeNode::new_directory(String::new());
            }
        };

        let path = if parent_path.is_empty() {
            entry.name.clone()
        } else {
            format!("{}\\{}", parent_path, entry.name)
        };

        if entry.is_directory {
            let mut node = TreeNode::new_directory(entry.name.clone());
            node.path = path.clone();

            // Alle Kinder dieses Ordners holen
            if let Some(child_refs) = self.children_index.get(&mft_reference) {
                for child_ref in child_refs {
                    // Vermeide Endlosrekursion: Root zeigt auf sich selbst
                    if *child_ref == mft_reference {
                        continue;
                    }
                    let child_node = self.build_subtree(*child_ref, &path);
                    node.add_child(child_node);
                }
            }

            // Nach Größe sortieren
            node.sort_by_size();

            node
        } else {
            let mut node = TreeNode::new_file(entry.name.clone(), entry.size);
            node.path = path;
            node
        }
    }

    /// Gibt die Top-N größten Ordner/Dateien zurück
    pub fn top_n(root: &TreeNode, n: usize) -> Vec<TreeNode> {
        let mut all_nodes: Vec<TreeNode> = Vec::new();
        Self::collect_nodes(root, &mut all_nodes);

        // Nach Größe sortieren
        all_nodes.sort_by(|a, b| b.total_size.cmp(&a.total_size));

        // Top N zurückgeben
        all_nodes.into_iter().take(n).collect()
    }

    /// Sammelt alle Knoten rekursiv (Hilfsfunktion)
    fn collect_nodes(node: &TreeNode, result: &mut Vec<TreeNode>) {
        result.push(node.clone());
        for child in &node.children {
            Self::collect_nodes(child, result);
        }
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

        assert_eq!(tree.children.len(), 1);
        assert_eq!(tree.children[0].name, "Users");
        assert_eq!(tree.children[0].total_size, 1024);
    }
}
