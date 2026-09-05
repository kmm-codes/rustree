# Ownership & Borrowing

Ownership ist DAS zentrale Konzept in Rust. Es macht Rust speichersicher ohne Garbage Collector.

## Die drei Regeln

1. **Jeder Wert hat genau einen Owner**
2. **Es kann nur einen Owner gleichzeitig geben**
3. **Wenn der Owner out of scope geht, wird der Wert gedroppt**

## In rustree

### FileEntry und Ownership

```rust
// In mft/types.rs
pub struct FileEntry {
    pub name: String,      // FileEntry BESITZT diesen String
    pub size: u64,
    // ...
}
```

Der `String` gehört dem `FileEntry`. Wenn wir den Entry löschen, wird auch der String freigegeben.

### HashMap und Ownership

```rust
// In tree/builder.rs
let mut entries: HashMap<u64, FileEntry> = HashMap::new();

// Entry wird IN die HashMap MOVED (nicht kopiert!)
entries.insert(mft_ref, entry);  // entry ist jetzt weg!

// Das würde nicht kompilieren:
// println!("{}", entry.name);  // Error: value used after move
```

### Borrowing im TreeBuilder

```rust
// Wir BORGEN uns die Referenz, statt zu ownen
for (mft_ref, entry) in &entries {
    //                   ^ Immutable borrow
    println!("{}: {}", mft_ref, entry.name);
}
// entries ist immer noch gültig!
```

## Warum ist das gut?

```rust
// OHNE Ownership (wie in C):
char* name = malloc(100);
strcpy(name, "test.txt");
free(name);
printf("%s", name);  // 💥 Use-after-free Bug!

// MIT Ownership (Rust):
let name = String::from("test.txt");
drop(name);
println!("{}", name);  // ❌ Kompiliert nicht!
```

Rust verhindert **zur Compile-Zeit**:
- Use-after-free
- Double-free
- Dangling pointers
- Data races

## Praktische Tipps

### Clone wenn nötig

```rust
let entry = entries.get(&mft_ref).unwrap();
let name_copy = entry.name.clone();  // Explizite Kopie
```

### Rc für geteiltes Ownership

```rust
use std::rc::Rc;

// Mehrere Owner möglich
let shared_tree = Rc::new(tree);
let also_owns_tree = Rc::clone(&shared_tree);
```

### RefCell für innere Mutabilität

```rust
use std::cell::RefCell;

// Mutieren trotz immutabler Referenz
let entries = RefCell::new(HashMap::new());
entries.borrow_mut().insert(key, value);
```

## Übung

Schau dir `src/tree/builder.rs` an:
- Wo wird Ownership transferiert?
- Wo wird geborgt?
- Warum gibt `build()` einen `TreeNode` zurück statt `&TreeNode`?
