# Installation

## Voraussetzungen

### 1. Rust installieren

Falls noch nicht geschehen, installiere Rust über [rustup.rs](https://rustup.rs):

```powershell
winget install Rustlang.Rustup
```

Oder lade den Installer von https://rustup.rs herunter.

Nach der Installation, öffne eine neue PowerShell und prüfe:

```powershell
rustc --version
cargo --version
```

### 2. Visual Studio Build Tools

Rust auf Windows benötigt die C++ Build Tools. Falls du Visual Studio hast, sind diese bereits installiert. Ansonsten:

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools
```

Bei der Installation wähle "C++ Build Tools" aus.

## Projekt klonen/herunterladen

```powershell
git clone https://github.com/kmm-codes/rustree.git
cd rustree
```

Oder erstelle das Projekt von Grund auf:

```powershell
cargo new rustree
cd rustree
```

## Dependencies installieren und bauen

```powershell
cargo build
```

Beim ersten Build lädt Cargo alle Dependencies herunter:
- `slint` - GUI Framework
- `windows` - Windows API Bindings
- `clap` - CLI Argument Parsing
- und mehr...

Das kann beim ersten Mal einige Minuten dauern!

## Release-Build (schneller)

Für einen optimierten Build:

```powershell
cargo build --release
```

Die fertige `.exe` findest du dann in `target/release/rustree.exe`.

## Häufige Probleme

### "linker not found"

Du brauchst die Visual Studio Build Tools. Siehe oben.

### Slint kompiliert nicht

Stelle sicher, dass du die neueste Rust-Version hast:

```powershell
rustup update
```

### Admin-Rechte für MFT-Zugriff

rustree braucht Administrator-Rechte um die MFT zu lesen. Starte die App als Administrator!

## Nächster Schritt

→ [Erster Start](first-run.md)
