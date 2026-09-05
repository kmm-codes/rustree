# Die NTFS Master File Table (MFT)

## Was ist die MFT?

Die MFT ist das Herzstück jedes NTFS-Dateisystems. Sie ist eine spezielle Datei, die **Metadaten über ALLE Dateien und Ordner** auf dem Laufwerk enthält.

```
┌─────────────────────────────────────────────────────────┐
│                    NTFS Laufwerk                         │
├─────────────────────────────────────────────────────────┤
│  Boot Sector (512 Bytes)                                 │
│  - Sagt wo die MFT ist                                   │
├─────────────────────────────────────────────────────────┤
│  Master File Table (MFT)                                 │
│  ┌─────────────────────────────────────────────────────┐│
│  │ Record 0: $MFT (die MFT selbst!)                    ││
│  │ Record 1: $MFTMirr (Backup)                         ││
│  │ Record 2: $LogFile                                   ││
│  │ Record 3: $Volume                                    ││
│  │ Record 4: $AttrDef                                   ││
│  │ Record 5: Root-Verzeichnis (C:\)  ← Hier starten wir││
│  │ Record 6: $Bitmap                                    ││
│  │ Record 7: $Boot                                      ││
│  │ ...                                                  ││
│  │ Record N: Deine Dateien und Ordner                  ││
│  └─────────────────────────────────────────────────────┘│
│                                                          │
│  Datei-Cluster (der eigentliche Dateiinhalt)            │
│                                                          │
└─────────────────────────────────────────────────────────┘
```

## MFT Record Struktur

Jeder MFT-Record ist normalerweise **1024 Bytes** groß:

```
┌────────────────────────────────────────┐
│ FILE Header (48 Bytes)                  │
│ - Signatur "FILE"                       │
│ - Flags (in use, directory)             │
│ - Offset zum ersten Attribut            │
├────────────────────────────────────────┤
│ $STANDARD_INFORMATION (0x10)            │
│ - Timestamps                            │
│ - DOS-Flags (hidden, system, etc.)      │
├────────────────────────────────────────┤
│ $FILE_NAME (0x30)                       │
│ - Dateiname (Unicode)                   │
│ - Parent-Referenz (→ übergeordneter     │
│   Ordner)                               │
├────────────────────────────────────────┤
│ $DATA (0x80)                            │
│ - Dateigröße                            │
│ - Cluster-Runs (wo die Daten liegen)    │
├────────────────────────────────────────┤
│ End Marker (0xFFFFFFFF)                 │
└────────────────────────────────────────┘
```

## Wie lesen wir die MFT?

### Schritt 1: Raw-Device öffnen

```rust
// Windows erlaubt direkten Laufwerkszugriff über spezielle Pfade
let path = "\\\\.\\C:";  // Das ist \\.\C: (backslash escaping)
```

Das ist wie das Öffnen einer normalen Datei, aber wir bekommen das **rohe Laufwerk** ohne Dateisystem-Abstraktion!

### Schritt 2: Boot-Sektor lesen

Die ersten 512 Bytes enthalten wichtige Informationen:

```rust
// Offset 11-12: Bytes pro Sektor (meist 512)
let bytes_per_sector = u16::from_le_bytes([buffer[11], buffer[12]]);

// Offset 13: Sektoren pro Cluster (variiert, oft 8)
let sectors_per_cluster = buffer[13];

// Offset 48-55: Start-Cluster der MFT
let mft_start = u64::from_le_bytes([
    buffer[48], buffer[49], buffer[50], buffer[51],
    buffer[52], buffer[53], buffer[54], buffer[55],
]);
```

### Schritt 3: MFT-Position berechnen

```rust
let mft_offset = mft_start * sectors_per_cluster * bytes_per_sector;
// Jetzt können wir zur MFT seekenn!
```

### Schritt 4: Records lesen und parsen

```rust
loop {
    let record = read_bytes(1024);  // Ein Record = 1024 Bytes

    if &record[0..4] != b"FILE" {
        break;  // Kein gültiger Record mehr
    }

    let entry = parse_record(&record)?;
    entries.insert(entry.mft_reference, entry);
}
```

## Die Parent-Referenz

Jeder Eintrag weiß, wer sein Parent ist:

```
MFT-Ref 5: Root (C:\)        parent: 5 (sich selbst)
MFT-Ref 100: Users           parent: 5 → gehört zu C:\
MFT-Ref 200: kevin           parent: 100 → gehört zu Users
MFT-Ref 300: Documents       parent: 200 → gehört zu kevin
```

Mit dieser Information bauen wir den Baum auf!

## Warum ist das so schnell?

| Methode | Operationen für 1M Dateien |
|---------|---------------------------|
| Rekursives Durchlaufen | ~1 Million Syscalls |
| MFT-Lesen | ~1 großer sequentieller Read |

Die MFT ist zusammenhängend auf der Festplatte → ein schneller linearer Scan!
