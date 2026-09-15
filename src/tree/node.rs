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

    /// Entfernt den Nachfahren an `chain` (Weg von IDs ab `self`, wie ihn
    /// `visible_chain_at` in main.rs liefert) aus dem Baum und gibt ihn
    /// zurück. Auf dem Weg dorthin werden `total_size`, `file_count` und
    /// `dir_count` aller Vorfahren um die Werte des entfernten Knotens
    /// verringert - so bleibt der Baum konsistent, ohne dass jemand ihn
    /// neu aufbauen muss (das würde bei Millionen Knoten spürbar dauern).
    ///
    /// Für den Aufrufer (GUI nach "Löschen"): `self` ist die Baumwurzel,
    /// `chain` die Kette zum gelöschten Knoten. Eine leere Kette (die
    /// Wurzel selbst) oder eine Kette, die im Baum nicht existiert, geben
    /// `None` zurück und lassen den Baum unangetastet.
    pub fn remove_descendant(&mut self, chain: &[u64]) -> Option<TreeNode> {
        let (&id, rest) = chain.split_first()?;

        let removed = if rest.is_empty() {
            // `id` ist ein direktes Kind von `self` - hier endet der Weg
            let index = self.children.iter().position(|child| child.id == id)?;
            self.children.remove(index)
        } else {
            // Weiter absteigen; der nächste Schritt existiert nur, wenn
            // die Kette zu einem echten Nachfahren gehört
            let child = self.children.iter_mut().find(|child| child.id == id)?;
            child.remove_descendant(rest)?
        };

        // Erst nach erfolgreichem Entfernen abziehen - bricht die Suche
        // irgendwo auf dem Weg ab (Kette falsch), bleibt dank des `?` oben
        // vorher nichts verändert.
        self.total_size -= removed.total_size;
        self.file_count -= removed.file_count;
        self.dir_count -= removed.dir_count;

        Some(removed)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Baut C: > Users > kevin > {photo.jpg, docs > notes.txt} sowie
    /// C: > pagefile.sys auf, mit IDs statt der Default-0, damit sich
    /// remove_descendant über eine echte Kette ansteuern lässt.
    fn sample_tree() -> TreeNode {
        let mut root = TreeNode::new_directory("C:".to_string());
        root.id = 1;

        let mut users = TreeNode::new_directory("Users".to_string());
        users.id = 2;

        let mut kevin = TreeNode::new_directory("kevin".to_string());
        kevin.id = 3;

        let mut photo = TreeNode::new_file("photo.jpg".to_string(), 2000);
        photo.id = 4;

        let mut docs = TreeNode::new_directory("docs".to_string());
        docs.id = 5;
        let mut notes = TreeNode::new_file("notes.txt".to_string(), 100);
        notes.id = 6;
        docs.add_child(notes);

        kevin.add_child(photo);
        kevin.add_child(docs);
        users.add_child(kevin);
        root.add_child(users);

        let mut pagefile = TreeNode::new_file("pagefile.sys".to_string(), 5000);
        pagefile.id = 7;
        root.add_child(pagefile);

        root
    }

    #[test]
    fn remove_descendant_updates_all_ancestors() {
        let mut root = sample_tree();

        // notes.txt (ID 6) liegt unter Users(2) > kevin(3) > docs(5)
        let removed = root
            .remove_descendant(&[2, 3, 5, 6])
            .expect("notes.txt sollte gefunden werden");
        assert_eq!(removed.name, "notes.txt");
        assert_eq!(removed.total_size, 100);

        // docs(5) hat kein Kind mehr, ist aber selbst noch da (nur die
        // Datei sollte entfernt worden sein, nicht ihr Elternordner)
        let docs = &root.children[0].children[0].children[1];
        assert_eq!(docs.name, "docs");
        assert!(docs.children.is_empty());
        assert_eq!(docs.total_size, 0);
        assert_eq!(docs.file_count, 0);

        // kevin(3), Users(2) und die Wurzel müssen die Größe/Zähler
        // ebenfalls um den entfernten Knoten verringert haben
        let kevin = &root.children[0].children[0];
        assert_eq!(kevin.total_size, 2000); // nur noch photo.jpg
        assert_eq!(kevin.file_count, 1);

        let users = &root.children[0];
        assert_eq!(users.total_size, 2000);
        assert_eq!(users.file_count, 1);

        assert_eq!(root.total_size, 2000 + 5000); // photo.jpg + pagefile.sys
        assert_eq!(root.file_count, 2);
        // dir_count sinkt nicht, weil eine Datei entfernt wurde, kein Ordner
        assert_eq!(root.dir_count, 4); // Users, kevin, docs + Wurzel selbst
    }

    #[test]
    fn remove_descendant_of_a_directory_drops_its_whole_subtree() {
        let mut root = sample_tree();

        // kevin (ID 3) mit allem Inhalt (photo.jpg + docs/notes.txt) löschen
        let removed = root
            .remove_descendant(&[2, 3])
            .expect("kevin sollte gefunden werden");
        assert_eq!(removed.name, "kevin");
        assert_eq!(removed.total_size, 2100);
        assert_eq!(removed.file_count, 2);

        let users = &root.children[0];
        assert!(users.children.is_empty());
        assert_eq!(users.total_size, 0);
        assert_eq!(users.file_count, 0);
        assert_eq!(users.dir_count, 1); // nur noch Users selbst

        assert_eq!(root.total_size, 5000); // nur noch pagefile.sys
        assert_eq!(root.file_count, 1);
        assert_eq!(root.dir_count, 2); // Users + Wurzel
    }

    #[test]
    fn remove_descendant_with_unknown_chain_returns_none_and_changes_nothing() {
        let mut root = sample_tree();
        let before_total = root.total_size;
        let before_file_count = root.file_count;
        let before_dir_count = root.dir_count;

        // ID 999 existiert nirgends im Baum
        assert!(root.remove_descendant(&[2, 999]).is_none());
        assert!(root.remove_descendant(&[999]).is_none());

        assert_eq!(root.total_size, before_total);
        assert_eq!(root.file_count, before_file_count);
        assert_eq!(root.dir_count, before_dir_count);
    }

    #[test]
    fn remove_descendant_with_empty_chain_returns_none() {
        let mut root = sample_tree();
        assert!(root.remove_descendant(&[]).is_none());
        assert_eq!(root.children.len(), 2); // Users und pagefile.sys unangetastet
    }
}
