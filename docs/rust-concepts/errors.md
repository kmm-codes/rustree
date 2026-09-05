# Error Handling in Rust

Rust hat kein `try/catch`. Stattdessen gibt es `Result<T, E>` und `Option<T>`.

## Result<T, E>

```rust
enum Result<T, E> {
    Ok(T),    // Erfolg mit Wert vom Typ T
    Err(E),   // Fehler mit Fehler vom Typ E
}
```

### In rustree: MftError

```rust
// In mft/types.rs
#[derive(Error, Debug)]
pub enum MftError {
    #[error("Zugriff verweigert - Admin-Rechte erforderlich")]
    AccessDenied,

    #[error("Laufwerk nicht gefunden: {0}")]
    DriveNotFound(String),

    #[error("Kein NTFS-Dateisystem: {0}")]
    NotNtfs(String),

    // ...
}
```

### Verwendung

```rust
fn read_boot_sector(drive: &str) -> Result<BootSectorInfo, MftError> {
    // Wenn etwas schiefgeht:
    if !is_ntfs {
        return Err(MftError::NotNtfs(drive.to_string()));
    }

    // Bei Erfolg:
    Ok(BootSectorInfo { ... })
}
```

## Der ? Operator

Statt:
```rust
let result = read_boot_sector(drive);
let info = match result {
    Ok(info) => info,
    Err(e) => return Err(e),
};
```

Einfach:
```rust
let info = read_boot_sector(drive)?;
//                                ^ Bei Err: früh zurückkehren
```

## Option<T>

Für Werte die möglicherweise nicht existieren:

```rust
enum Option<T> {
    Some(T),  // Wert vorhanden
    None,     // Kein Wert
}
```

### In rustree: Record Parsing

```rust
// In mft/parser.rs
pub fn parse_record(record: &[u8]) -> Option<FileEntry> {
    // Ungültiger Record = None
    if &record[0..4] != b"FILE" {
        return None;
    }

    // Gültiger Record = Some(entry)
    Some(FileEntry { ... })
}
```

## Kombinieren: Option → Result

```rust
// Option in Result umwandeln
let entry = parse_record(&record)
    .ok_or(MftError::InvalidRecord)?;
//  ^^^^^^ None wird zu Err(MftError::InvalidRecord)
```

## Best Practices

### 1. Eigene Error-Typen mit thiserror

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MftError {
    #[error("Windows API Fehler: {0}")]
    WindowsError(#[from] windows::core::Error),
    //           ^^^^^ Automatische Konvertierung!
}
```

### 2. Errors nach oben propagieren

```rust
fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_gui()?;  // Fehler werden nach oben gegeben
    Ok(())
}
```

### 3. unwrap() nur im Prototyping

```rust
// NICHT in Produktion:
let file = File::open("test.txt").unwrap();  // 💥 Panic bei Fehler

// BESSER:
let file = File::open("test.txt")?;  // Fehler wird behandelt
```

## Übung

Schau dir `src/mft/reader.rs` an:
- Wie werden Windows-API-Fehler behandelt?
- Wo wird `?` verwendet?
- Was passiert bei `Err(MftError::NotNtfs(...))`?
