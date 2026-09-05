# Traits - Rusts Interface-System

Traits sind wie Interfaces in anderen Sprachen - sie definieren gemeinsames Verhalten.

## Ein Trait definieren

```rust
trait Scannable {
    fn scan(&self) -> Vec<FileEntry>;
    fn name(&self) -> &str;
}
```

## Einen Trait implementieren

```rust
impl Scannable for MftReader {
    fn scan(&self) -> Vec<FileEntry> {
        // MFT-spezifische Implementierung
    }

    fn name(&self) -> &str {
        "MFT Scanner"
    }
}
```

## Standard-Traits in rustree

### Debug - Für println! Debugging

```rust
#[derive(Debug)]  // Automatisch implementiert
pub struct FileEntry {
    pub name: String,
    pub size: u64,
}

// Jetzt geht:
println!("{:?}", entry);  // FileEntry { name: "test.txt", size: 1024 }
```

### Clone - Werte kopieren

```rust
#[derive(Clone)]
pub struct TreeNode {
    pub name: String,
    pub children: Vec<TreeNode>,
}

let copy = node.clone();  // Tiefe Kopie
```

### Ord & PartialOrd - Sortierbarkeit

```rust
// In tree/node.rs
impl Ord for TreeNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // Größere Dateien zuerst
        other.total_size.cmp(&self.total_size)
    }
}

// Jetzt geht:
nodes.sort();  // Sortiert nach Größe!
```

### From/Into - Konvertierungen

```rust
impl From<windows::core::Error> for MftError {
    fn from(err: windows::core::Error) -> Self {
        MftError::WindowsError(err)
    }
}

// Automatische Konvertierung:
let mft_error: MftError = windows_error.into();
```

## Trait Bounds

```rust
// T muss Debug implementieren
fn print_all<T: Debug>(items: &[T]) {
    for item in items {
        println!("{:?}", item);
    }
}

// Mehrere Bounds:
fn process<T: Clone + Debug + Send>(item: T) {
    // ...
}
```

## Trait Objects

```rust
// Dynamischer Dispatch mit dyn
fn handle_error(e: &dyn std::error::Error) {
    println!("Fehler: {}", e);
}

// Box<dyn Trait> für Ownership
fn get_scanner() -> Box<dyn Scannable> {
    Box::new(MftReader::new())
}
```

## In rustree

### Error-Trait für MftError

```rust
// thiserror macht das automatisch:
#[derive(Error, Debug)]
pub enum MftError {
    #[error("Zugriff verweigert")]
    AccessDenied,
}
// MftError implementiert jetzt std::error::Error!
```

### Display für TreeNode

```rust
impl std::fmt::Display for TreeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.size_string())
    }
}

// Jetzt geht:
println!("{}", node);  // "Documents (15.2 GB)"
```

## Übung

1. Welche Traits implementiert `FileEntry`?
2. Warum braucht `TreeNode` das `Ord` Trait?
3. Wie würdest du ein `Displayable` Trait für die GUI implementieren?
