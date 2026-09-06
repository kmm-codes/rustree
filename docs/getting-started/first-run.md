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
│  Name              Größe ▼ │  Treemap  C:\Users     [Hoch] │
│  ─────────────────         │  ┌──────────────┬──────┬────┐ │
│  ▶ 📁 Windows     25.3 GB  │  │              │      │    │ │
│  ▶ 📁 Program...  15.7 GB  │  │              ├──────┴────┤ │
│  ▼ 📁 Users       45.2 GB  │  │              │ ▪▪▪▪▪▪▪▪▪ │ │
│    ▶ 📁 kevin     42.1 GB  │  ├──────┬───────┤ ▪▪▪▪▪▪▪▪▪ │ │
│    ▶ 📁 Public     3.1 GB  │  │      │       │ ▪▪▪▪▪▪▪▪▪ │ │
│  📄 pagefile.sys   8.0 GB  │  └──────┴───────┴───────────┘ │
│                            │  C:\Users\kevin\video.mp4 - 1 GB│
└────────────────────────────┴────────────────────────────────┘
```

### Bedienung

1. **Administratorrechte**: Beim Start fragt rustree, ob es sich mit
   Administratorrechten neu starten soll - ohne sie lässt sich die MFT
   nicht lesen
2. **Laufwerk wählen**: Die Startansicht listet die lokalen Laufwerke mit
   Größe, freiem Platz und Belegung; ein Klick startet den Scan. "Ordner
   wählen..." scannt das Laufwerk ebenfalls komplett und zeigt nur den
   Ordner. Ein Wechsel in der Kopfzeile scannt sofort, "Neu scannen"
   wiederholt den letzten Scan
3. **Ordner aufklappen**: Klick auf das Dreieck um Unterordner zu sehen
4. **Sortieren**: Klick auf "Name" oder "Größe" im Spaltenkopf, ein
   zweiter Klick dreht die Richtung um
5. **Treemap**: Jede Kachel ist eine Datei, ihre Fläche die Größe, ihre
   Farbe die Dateiendung. Die Maus darüber zeigt Pfad und Größe, ein Klick
   markiert die Datei im Baum, ein Doppelklick zoomt in den Ordner - "Hoch"
   führt wieder eine Ebene zurück

## Was passiert beim Scan?

1. **MFT öffnen**: Das Laufwerk wird als Raw-Device geöffnet
2. **Boot-Sektor lesen**: Hier steht wo die MFT beginnt
3. **MFT parsen**: Alle Records werden sequentiell gelesen
4. **Baum aufbauen**: Aus den Records wird die Ordnerstruktur gebaut
5. **Anzeigen**: Die GUI zeigt den Baum und berechnet Größen

Warum ist das so schnell? Weil wir nicht jeden Ordner einzeln öffnen müssen - alle Informationen stehen bereits in der MFT!

## Nächster Schritt

→ [Architektur-Übersicht](../architecture/overview.md)
