# Erster Start

## Die App starten

### Wichtig: Admin-Rechte

rustree benötigt **Administrator-Rechte** um die NTFS MFT zu lesen. Die Release-Version fordert diese automatisch an (Windows UAC-Dialog).

### GUI-Modus (Standard)

**Development (mit cargo):**
```powershell
# PowerShell als Administrator öffnen!
cargo run --release
```

**Release-Build:**
```powershell
# Einmal bauen
cargo build --release

# Die EXE fordert automatisch Admin-Rechte an
.\target\release\rustree.exe
```

### CLI-Modus

```powershell
# Mit cargo
cargo run --release -- --cli

# Oder direkt
.\target\release\rustree.exe --cli
```

Mit spezifischem Laufwerk:

```powershell
.\target\release\rustree.exe --cli --drive D:
# oder kurz:
.\target\release\rustree.exe --cli -d D:
```

## Die GUI verstehen

```
┌─────────────────────────────────────────────────────────────┐
│  Laufwerk: [C: ▼]  [Scannen]              Status: Bereit    │
├────────────────────────────┬────────────────────────────────┤
│  Verzeichnisse             │  Treemap                       │
│  ─────────────────         │  ────────                      │
│  ▶ 📁 Windows     25.3 GB  │                                │
│  ▶ 📁 Program...  15.7 GB  │      ┌────────────────────┐    │
│  ▼ 📁 Users       45.2 GB  │      │                    │    │
│    ▶ 📁 kevin     42.1 GB  │      │   (Visualisierung  │    │
│    ▶ 📁 Public     3.1 GB  │      │    kommt später)   │    │
│  📄 pagefile.sys   8.0 GB  │      │                    │    │
│                            │      └────────────────────┘    │
└────────────────────────────┴────────────────────────────────┘
```

### Bedienung

1. **Laufwerk wählen**: Dropdown-Menü oben links
2. **Scannen**: Klick auf "Scannen" startet den MFT-Scan
3. **Ordner aufklappen**: Klick auf ▶ um Unterordner zu sehen
4. **Größen**: Rechts siehst du die Gesamtgröße jedes Ordners

## Was passiert beim Scan?

1. **MFT öffnen**: Das Laufwerk wird als Raw-Device geöffnet
2. **Boot-Sektor lesen**: Hier steht wo die MFT beginnt
3. **MFT parsen**: Alle Records werden sequentiell gelesen
4. **Baum aufbauen**: Aus den Records wird die Ordnerstruktur gebaut
5. **Anzeigen**: Die GUI zeigt den Baum und berechnet Größen

Warum ist das so schnell? Weil wir nicht jeden Ordner einzeln öffnen müssen - alle Informationen stehen bereits in der MFT!

## Nächster Schritt

→ [Architektur-Übersicht](../architecture/overview.md)
