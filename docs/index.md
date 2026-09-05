# rustree - Lerne Rust mit einem echten Projekt

Willkommen bei **rustree** - einem schnellen Disk Space Analyzer für Windows, geschrieben in Rust!

## Was ist rustree?

rustree ist ein Tool ähnlich wie WinDirStat oder TreeSize, aber:
- **Schneller**: Nutzt die NTFS Master File Table (MFT) für Sekunden-schnelle Scans
- **Modern**: Geschrieben in Rust mit Slint für die GUI
- **Lehrreich**: Der Code ist anfängerfreundlich dokumentiert

## Aktueller Status

Der laufende Stand wird über [GitHub Issues](https://github.com/kmm-codes/rustree/issues)
und die [Milestones](https://github.com/kmm-codes/rustree/milestones) je Phase verfolgt.

| Phase | Status | Beschreibung |
|-------|--------|--------------|
| 1. Projekt-Setup | ✅ Fertig | Cargo, Slint, Dependencies |
| 2. MFT-Reader | ✅ Fertig | Raw-Disk-Zugriff, Boot-Sektor, Record-Parsing |
| 3. Baum-Datenstruktur | ✅ Fertig | TreeNode, TreeBuilder, Größenaggregation |
| 4. GUI Baumansicht | 🔄 In Arbeit | Aufklappbare Liste, Background-Threading |
| 5. Treemap | ⏳ Ausstehend | Squarified-Algorithmus |
| 6. CLI & Polish | ⏳ Ausstehend | Export, Feinschliff |

### Letzte Änderungen

- **Vollständiger MFT-Scan**: Der Reader folgt den Data Runs der `$MFT`, wendet Fixups an und bevorzugt lange Windows-Namen
- **Baumliste**: Namen werden angezeigt und bei Platzmangel gekürzt, Skia-Renderer für saubere Schrift
- **Setup und Dev-Loop**: `scripts/release.ps1` baut ein NSIS-Setup, `scripts/update.ps1` ersetzt die installierte EXE
- **20 Unit-Tests**: Parser (Fixups, Data Runs, Namensauswahl), TreeNode, TreeBuilder, format_size

## Warum dieses Projekt?

Dieses Projekt wurde erstellt, um Rust zu lernen. Jede Code-Datei ist ausführlich kommentiert und erklärt:
- Was der Code macht
- Warum er so geschrieben ist
- Welche Rust-Konzepte verwendet werden

## Projekt-Struktur

```
rustree/
├── src/
│   ├── main.rs          # Einstiegspunkt, GUI & CLI
│   ├── lib.rs           # Library-Exports für Tests
│   ├── mft/             # MFT-Zugriff (das Herzstück)
│   │   ├── mod.rs       # Modul-Definition
│   │   ├── reader.rs    # Raw-Disk-Zugriff
│   │   ├── parser.rs    # MFT-Record-Parsing
│   │   └── types.rs     # Datentypen (FileEntry, MftError)
│   └── tree/            # Baum-Datenstruktur
│       ├── mod.rs       # Modul-Definition
│       ├── node.rs      # TreeNode mit Größenaggregation
│       └── builder.rs   # Baum-Konstruktion aus MFT
├── ui/
│   └── main.slint       # GUI-Definition (Slint)
├── tests/
│   └── ui_tests.rs      # Integration-Tests
├── docs/                # Diese Dokumentation
├── installer/
│   └── rustree.nsi      # NSIS-Setup (baut scripts/release.ps1)
├── scripts/
│   ├── update.ps1       # Bauen, installierte EXE ersetzen, neu starten
│   └── release.ps1      # Gates, Release-Build, Setup
├── rustree.manifest     # Windows Admin-Manifest
└── build.rs             # Build-Skript für Slint, Manifest & Versionsinfo
```

## Los geht's!

1. [Installation](getting-started/installation.md) - Rust und das Projekt einrichten
2. [Erster Start](getting-started/first-run.md) - Die App starten
3. [Architektur](architecture/overview.md) - Wie das Projekt aufgebaut ist

## Rust-Konzepte in diesem Projekt

| Konzept | Wo verwendet | Dokumentation |
|---------|--------------|---------------|
| Ownership & Borrowing | Überall | [Lernen](rust-concepts/ownership.md) |
| Error Handling | `MftError`, `Result` | [Lernen](rust-concepts/errors.md) |
| Traits | `Ord`, `PartialEq` | [Lernen](rust-concepts/traits.md) |
| Generics | `HashMap<K,V>` | Kommt noch |
| FFI/Windows API | `mft/reader.rs` | Kommt noch |
| Threading | `Arc<Mutex<>>`, `std::thread` | [Lernen](rust-concepts/threading.md) |
| Closures | Callbacks in Slint | Kommt noch |
