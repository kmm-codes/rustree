# Architektur-Übersicht

## Das große Bild

```
┌─────────────────────────────────────────────────────────────────┐
│                           rustree                                │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ┌──────────┐     ┌──────────┐     ┌──────────┐               │
│   │   GUI    │     │   CLI    │     │          │               │
│   │ (Slint)  │     │ (clap)   │     │   ...    │               │
│   └────┬─────┘     └────┬─────┘     └──────────┘               │
│        │                │                                       │
│        └───────┬────────┘                                       │
│                ▼                                                 │
│   ┌─────────────────────────────────────────┐                   │
│   │            Application Layer             │                   │
│   │         (main.rs - Koordination)         │                   │
│   └─────────────────┬───────────────────────┘                   │
│                     │                                            │
│        ┌────────────┼────────────┐                              │
│        ▼            ▼            ▼                              │
│   ┌─────────┐ ┌──────────┐ ┌──────────┐                        │
│   │   MFT   │ │   Tree   │ │ Treemap  │                        │
│   │ Module  │ │  Module  │ │  Module  │                        │
│   └────┬────┘ └────┬─────┘ └──────────┘                        │
│        │           │                                             │
│        ▼           │                                             │
│   ┌─────────┐      │                                            │
│   │ Windows │      │                                            │
│   │   API   │◄─────┘                                            │
│   └────┬────┘                                                    │
│        │                                                         │
│        ▼                                                         │
│   ┌─────────────────────────────────────────┐                   │
│   │              NTFS Laufwerk               │                   │
│   │     (MFT = Master File Table)            │                   │
│   └─────────────────────────────────────────┘                   │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Die Module

### 1. MFT-Modul (`src/mft/`)

Das Herzstück! Liest die NTFS Master File Table direkt aus.

```rust
// Hauptkomponenten
MftReader  - Öffnet das Laufwerk, liest die rohen Bytes
MftParser  - Interpretiert die Bytes als Datei-Einträge
FileEntry  - Datenstruktur für eine Datei/Ordner
```

**Warum direkt MFT lesen?**
- Normal: Jeder Ordner = ein Syscall → langsam bei vielen Ordnern
- MFT: Ein sequentieller Read → alle Dateien in Sekunden!

### 2. Tree-Modul (`src/tree/`)

Baut aus den flachen MFT-Daten eine Baumstruktur.

```rust
// Hauptkomponenten
TreeNode     - Ein Knoten (Datei oder Ordner)
TreeBuilder  - Konstruiert den Baum aus Vec<FileEntry> über einen dichten Index
```

**Der Trick:**
Die MFT gibt uns nur `parent_reference` - wir müssen selbst den Baum bauen!

### 3. Treemap-Modul (`src/treemap/`)

Visualisiert den Baum als Rechteck-Diagramm: jeder Knoten bekommt eine
Fläche proportional zu seiner Größe, die Kinder teilen die Fläche ihres
Ordners (Squarified-Layout nach Bruls, Huizing und van Wijk).

```rust
// Hauptkomponenten
layout()     - Verteilt eine Fläche auf Größen, möglichst quadratisch
Treemap      - Zeichnet den Baum in einen RGB-Puffer und merkt sich je
               Rechteck den Weg zum Knoten (für Hover und Klick)
```

**Der Trick:** Millionen Knoten passen nicht als GUI-Elemente auf den
Bildschirm. Die Treemap wird deshalb als fertiges Bild in der Größe des
Anzeigebereichs gezeichnet; Knoten unter einem Pixel werden samt Teilbaum
übersprungen. Der Aufwand hängt an der Bildgröße, nicht an der Dateizahl.

### 4. UI-Modul (`ui/`)

Slint-basierte GUI mit:
- Aufklappbarer Baumansicht
- Treemap-Visualisierung
- Drive-Auswahl

## Datenfluss

```
1. User klickt "Scannen"
        │
        ▼
2. MftReader öffnet \\.\C:
        │
        ▼
3. Boot-Sektor → MFT-Position finden
        │
        ▼
4. MftParser liest jeden Record
        │
        ▼
5. Vec<FileEntry> mit allen Dateien (Erweiterungs-Records zusammengeführt)
        │
        ▼
6. TreeBuilder baut Baum auf
        │
        ▼
7. TreeNode mit aggregierten Größen
        │
        ▼
8. GUI zeigt Baum + Treemap
```

## Rust-Konzepte pro Modul

| Modul | Rust-Konzepte |
|-------|---------------|
| MFT | unsafe (Windows API), FFI, Result/Error |
| Tree | Dichte Indizes statt HashMap, Rekursion, Ownership, rayon |
| Treemap | f64-Geometrie, Rekursion mit Abbruch, Pixelpuffer, Lifetimes |
| UI | Callbacks, Arc/Mutex, Threading, Closures |

## Threading-Architektur

Die App verwendet Multi-Threading um die UI responsive zu halten:

```
┌─────────────────────────────────────────────────────────────┐
│                      Main Thread (UI)                        │
│  - Slint Event Loop                                          │
│  - Reagiert auf User-Input                                   │
│  - Aktualisiert GUI                                          │
└─────────────────────────┬───────────────────────────────────┘
                          │
                          │ on_scan_drive()
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                    Background Thread                         │
│  - MFT-Scan (kann mehrere Sekunden dauern)                  │
│  - Sendet Progress via invoke_from_event_loop()             │
│  - Sendet Ergebnis zurück zum Main Thread                   │
└─────────────────────────────────────────────────────────────┘
```

**Wichtige Rust-Konzepte:**

- `Arc<Mutex<T>>` - Thread-sicherer gemeinsamer State
- `std::thread::spawn` - Neuen Thread starten
- `slint::invoke_from_event_loop` - Sicher vom Background zum UI-Thread kommunizieren
- `Clone` - Daten zwischen Threads kopieren (da kein shared borrowing möglich)

## Tests

Das Projekt enthält Unit- und Integration-Tests:

```powershell
# Alle Tests ausführen
cargo test

# Nur UI-Tests
cargo test --test ui_tests
```

**Aktuelle Tests:**
- `test_tree_entry_creation` - TreeEntry-Struct
- `test_tree_entry_model` - Slint VecModel
- `test_format_size` - Größenformatierung (KB, MB, GB, TB)
- `test_tree_node_creation` - TreeNode für Dateien/Ordner
- `test_tree_node_add_child` - Kind hinzufügen, Größenaggregation
- `test_tree_builder` - Baum aus MFT-Entries bauen
- `test_tree_sorting` - Sortierung nach Größe
- `test_top_n` - Top-N größte Einträge
