# rustree

Schneller NTFS Disk Space Analyzer für Windows, geschrieben in Rust mit einer
Slint-GUI. Statt den Verzeichnisbaum rekursiv zu durchlaufen, liest rustree die
Master File Table (MFT) direkt und baut daraus den Größenbaum. Dafür braucht es
Administrator-Rechte, die die EXE per Manifest selbst anfordert.

Das Projekt ist zugleich ein Rust-Lernprojekt: der Code ist ausführlich
kommentiert, die Dokumentation unter `docs/` (mkdocs) erklärt Architektur und
Rust-Konzepte.

## Bauen und starten

Voraussetzungen: Rust (rustup) und die Visual Studio Build Tools (C++), siehe
[docs/getting-started/installation.md](docs/getting-started/installation.md).

```powershell
cargo build                                # Debug-Build
cargo run --release                        # GUI, fordert Admin-Rechte an
cargo run --release -- --cli --drive C:    # CLI-Modus
cargo test
```

## Entwicklungs-Loop: installierte Version aktualisieren

`scripts/update.ps1` baut, ersetzt die installierte EXE unter
`%LOCALAPPDATA%\Programs\rustree` und startet die App neu. Läuft gerade eine
Instanz, wird sie hinter einem UAC-Dialog beendet, weil die App elevated läuft.
Ohne vorheriges Setup legt das Skript Verzeichnis und Startmenü-Eintrag selbst
an.

```powershell
pwsh scripts/update.ps1              # bauen + ersetzen + neu starten
pwsh scripts/update.ps1 -Release     # dasselbe mit optimiertem Build
pwsh scripts/update.ps1 -SkipBuild   # vorhandene EXE aus target/ verwenden
pwsh scripts/update.ps1 -NoLaunch    # nur ersetzen, nicht starten
pwsh scripts/update.ps1 -DryRun      # nur anzeigen, was passieren würde
```

## Release: Setup bauen

`scripts/release.ps1` prüft die Version aus `Cargo.toml` (kein bereits
getaggter Stand), lässt die Gates laufen (`cargo fmt --check`,
`cargo clippy -D warnings`, `cargo test`), baut den Release-Build und
kompiliert mit NSIS das Setup nach
`release/<version>/rustree_<version>_x64-setup.exe` samt `SHA256SUMS.txt`.

Das Setup installiert pro Benutzer nach `%LOCALAPPDATA%\Programs\rustree`,
legt einen Startmenü-Eintrag an und registriert die App unter
Einstellungen > Apps mit Deinstaller. Es ist dasselbe Verzeichnis, das
`update.ps1` aktualisiert.

```powershell
pwsh scripts/release.ps1              # Version prüfen + Gates + Build + Setup
pwsh scripts/release.ps1 -SkipGates   # nur bauen
```

NSIS wird auf dem PATH, im Standard-Installationsverzeichnis und in der Kopie
von Tauri unter `%LOCALAPPDATA%\tauri\NSIS` gesucht; sonst
`winget install NSIS.NSIS`.

Neue Version: `version` in `Cargo.toml` erhöhen, committen und den Commit mit
`v<version>` taggen.

## Messen ohne Admin-Rechte

Ein einmaliger Lauf als Administrator schreibt die rohen MFT-Records in eine
Datei; danach lassen sich Parser und Baumaufbau beliebig oft ohne Elevation
messen (Examples bekommen kein Admin-Manifest):

```powershell
rustree --cli --drive C: --dump-mft F:\dev\mft-c.bin          # als Administrator
cargo run --release --example scan_bench -- F:\dev\mft-c.bin 3 # drei Läufe
cargo run --example ui_preview                                 # GUI mit Beispieldaten
```

Die Statuszeile der GUI und die CLI zeigen die Dauer getrennt nach Scan
(MFT lesen und parsen) und Baum (Verzeichnisbaum aufbauen).

Zum Ausprobieren auf anderer Hardware lassen sich zwei Stellschrauben des
Lesers per Umgebungsvariable setzen: `RUSTREE_READERS` (gleichzeitige
Lesezugriffe, Standard 4) und `RUSTREE_CHUNK_MB` (Blockgröße, Standard 8).
Auf einem NVMe-Laufwerk mit 4,4 GB MFT: 1 Leser 3,4 s, 4 Leser 2,3 s Scan.

## Aufbau

```
rustree/
  src/
    main.rs           Einstiegspunkt, GUI & CLI
    lib.rs            Library-Exports für Tests
    mft/              MFT-Zugriff: Raw-Disk-Reader, Record-Parser, Typen
    tree/             Baum-Datenstruktur mit Größenaggregation
    treemap/          Squarified-Layout und Bild der Treemap
  ui/main.slint       GUI-Definition (Slint)
  tests/              Integrationstests
  examples/           ui_preview (GUI ohne Scan), scan_bench (MFT-Dump messen)
  installer/          NSIS-Setup (rustree.nsi)
  scripts/            update.ps1, release.ps1
  docs/               mkdocs-Dokumentation
  rustree.manifest    Windows-Manifest: Admin-Elevation, DPI-Awareness
  build.rs            Slint kompilieren, Manifest und Versionsinfo einbetten
```

## Dokumentation

`docs/` ist eine mkdocs-Site: `pip install mkdocs-material`, dann
`mkdocs serve`.
