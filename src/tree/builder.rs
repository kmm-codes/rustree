//! TreeBuilder - Konstruiert den Verzeichnisbaum aus MFT-Einträgen
//!
//! # Algorithmus
//!
//! 1. Alle MFT-Einträge liegen in einem Vektor, jeder kennt seine
//!    MFT-Referenz und die seines Parents
//! 2. Wir starten beim Root-Verzeichnis (MFT-Referenz 5)
//! 3. Für jeden Ordner finden wir alle Kinder (Einträge mit parent_reference == unsere Referenz)
//! 4. Rekursiv bauen wir so den Baum auf
//! 5. Zum Schluss wird der fertige Baum einmal sortiert
//!
//! # Performance-Tricks
//!
//! Für Schritt 3 braucht es einen Index Parent -> Kinder. Statt einer
//! HashMap mit einem `Vec` pro Ordner (700.000 Allokationen, Hashen von
//! Millionen Schlüsseln) sind es drei flache Arrays, wie bei einer dünn
//! besetzten Matrix im CSR-Format:
//!
//! - `index_of[mft_reference]` = Position des Eintrags im Vektor
//! - `child_start[i]..child_start[i+1]` = Bereich in `children` für Eintrag i
//! - `children` = Positionen der Kinder, nach Parent gruppiert
//!
//! Zwei lineare Durchläufe (zählen, einsortieren), keine Suche, kein
//! Hashen: Zählsortierung nach Parent.
//!
//! Sortiert wird genau einmal, von der Wurzel aus: würde jeder Ordner beim
//! Aufbau seinen Teilbaum sortieren, wäre ein Ordner in Tiefe 12 zwölfmal
//! sortiert. Und kein Knoten trägt seinen vollen Pfad - bei Millionen
//! Einträgen wären das hunderte MB an Strings, die fast nie gebraucht werden.

use super::node::TreeNode;
use crate::mft::FileEntry;

/// MFT-Referenz des Root-Verzeichnisses
const ROOT_REFERENCE: u64 = 5;

/// Markierung in `index_of` für Referenzen ohne Eintrag
const NO_ENTRY: u32 = u32::MAX;

/// Der TreeBuilder konstruiert den Baum aus MFT-Daten
pub struct TreeBuilder {
    /// Alle Dateien und Ordner
    entries: Vec<FileEntry>,

    /// MFT-Referenz -> Position in `entries` (oder `NO_ENTRY`)
    index_of: Vec<u32>,

    /// Kinder von Eintrag i: `children[child_start[i]..child_start[i + 1]]`
    child_start: Vec<u32>,

    /// Positionen aller Kinder, nach Parent gruppiert
    children: Vec<u32>,
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
    ///
    /// Die Reihenfolge ist egal; mehr als `u32::MAX` Einträge werden nicht
    /// unterstützt (eine MFT dieser Größe wäre vier Terabyte).
    pub fn new(entries: Vec<FileEntry>) -> Self {
        assert!(entries.len() < NO_ENTRY as usize, "zu viele Einträge");

        // Referenz -> Position
        let max_reference = entries
            .iter()
            .map(|entry| entry.mft_reference)
            .max()
            .unwrap_or(0);
        let mut index_of = vec![NO_ENTRY; max_reference as usize + 1];
        for (position, entry) in entries.iter().enumerate() {
            index_of[entry.mft_reference as usize] = position as u32;
        }

        // Kinder pro Parent zählen; Einträge ohne bekannten Parent (oder mit
        // sich selbst als Parent, wie die Wurzel) hängen nirgends
        let parent_of = |entry: &FileEntry| -> Option<usize> {
            let parent = *index_of.get(entry.parent_reference as usize)?;
            (parent != NO_ENTRY && entry.parent_reference != entry.mft_reference)
                .then_some(parent as usize)
        };
        let mut child_start = vec![0u32; entries.len() + 1];
        for entry in &entries {
            if let Some(parent) = parent_of(entry) {
                child_start[parent + 1] += 1;
            }
        }

        // Präfixsumme: aus Anzahlen werden Startpositionen
        for i in 1..child_start.len() {
            child_start[i] += child_start[i - 1];
        }

        // Einsortieren; `cursor` merkt sich pro Parent die nächste freie Stelle
        let mut children = vec![0u32; child_start[entries.len()] as usize];
        let mut cursor = child_start.clone();
        for (position, entry) in entries.iter().enumerate() {
            if let Some(parent) = parent_of(entry) {
                children[cursor[parent] as usize] = position as u32;
                cursor[parent] += 1;
            }
        }

        Self {
            entries,
            index_of,
            child_start,
            children,
        }
    }

    /// Baut den Baum ab dem Root-Verzeichnis auf, sortiert nach Größe
    ///
    /// Verbraucht den Builder: die Namen wandern in den Baum, statt kopiert
    /// zu werden.
    pub fn build(mut self) -> TreeNode {
        let mut root = match self.index_of.get(ROOT_REFERENCE as usize).copied() {
            Some(position) if position != NO_ENTRY => self.build_subtree(position as usize),
            _ => {
                // Fallback: Wurzel ohne Eintrag
                let mut node = TreeNode::new_directory(String::new());
                node.id = ROOT_REFERENCE;
                node
            }
        };
        root.sort_by_size();
        root
    }

    /// Baut einen Teilbaum rekursiv auf (unsortiert)
    fn build_subtree(&mut self, position: usize) -> TreeNode {
        let entry = &mut self.entries[position];
        let name = std::mem::take(&mut entry.name);
        let id = entry.mft_reference;

        if !entry.is_directory {
            let mut node = TreeNode::new_file(name, entry.size);
            node.id = id;
            return node;
        }

        let mut node = TreeNode::new_directory(name);
        node.id = id;

        let (start, end) = (
            self.child_start[position] as usize,
            self.child_start[position + 1] as usize,
        );
        node.children.reserve_exact(end - start);
        for i in start..end {
            let child = self.children[i] as usize;
            node.add_child(self.build_subtree(child));
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
        let entries = vec![
            // Root (MFT 5), zeigt auf sich selbst
            FileEntry::new(5, 5, String::new(), 0, true),
            // Ordner "Users" (MFT 100)
            FileEntry::new(100, 5, "Users".to_string(), 0, true),
            // Datei "test.txt" in Users (MFT 101)
            FileEntry::new(101, 100, "test.txt".to_string(), 1024, false),
        ];

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
        // Kinder vor ihren Parents, um die Reihenfolge-Unabhängigkeit zu prüfen
        let entries = vec![
            FileEntry::new(22, 11, "c".to_string(), 100, false),
            FileEntry::new(20, 10, "a".to_string(), 10, false),
            FileEntry::new(21, 11, "b".to_string(), 1, false),
            FileEntry::new(10, 5, "klein".to_string(), 0, true),
            FileEntry::new(11, 5, "gross".to_string(), 0, true),
            FileEntry::new(5, 5, String::new(), 0, true),
        ];

        let tree = TreeBuilder::new(entries).build();
        let names: Vec<&str> = tree.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["gross", "klein"]);
        let inner: Vec<&str> = tree.children[0].children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(inner, vec!["c", "b"]);
        assert_eq!(tree.file_count, 3);
        assert_eq!(tree.dir_count, 3);
    }

    #[test]
    fn orphans_and_missing_root_do_not_panic() {
        // Parent 999 existiert nicht, Root fehlt ganz
        let entries = vec![FileEntry::new(10, 999, "lost".to_string(), 5, false)];
        let tree = TreeBuilder::new(entries).build();
        assert_eq!(tree.id, 5);
        assert!(tree.children.is_empty());

        let tree = TreeBuilder::new(Vec::new()).build();
        assert_eq!(tree.total_size, 0);
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
