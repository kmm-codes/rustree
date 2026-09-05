//! MFT Reader - Direkter Zugriff auf die NTFS Master File Table
//!
//! # Wie funktioniert der Raw-Disk-Zugriff?
//!
//! Windows erlaubt das direkte Lesen von Laufwerken über spezielle Pfade:
//! - `\\.\C:` - Öffnet Laufwerk C: als Raw-Device
//!
//! Dafür sind Administrator-Rechte erforderlich!
//!
//! # Die Schritte zum MFT-Lesen:
//!
//! 1. Laufwerk als Raw-Device öffnen
//! 2. Boot-Sektor lesen (erste 512 Bytes): Sektor-, Cluster- und
//!    Record-Größe sowie der Start-Cluster der MFT
//! 3. Record 0 lesen - er beschreibt die MFT selbst. Sein `$DATA`-Attribut
//!    listet die Data Runs (die Fragmente der MFT auf der Platte) und ihre
//!    Gesamtgröße, aus der sich die Anzahl der Records ergibt
//! 4. Fragment für Fragment in großen Blöcken lesen, jeden Record per Fixup
//!    korrigieren und parsen

use super::parser::{attribute_types, MftParser};
use super::types::{FileEntry, MftError};
use std::collections::HashMap;

#[cfg(target_os = "windows")]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ, HANDLE},
        Storage::FileSystem::{
            CreateFileW, ReadFile, SetFilePointerEx, FILE_ATTRIBUTE_NORMAL, FILE_BEGIN,
            FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
    },
};

/// Blockgröße beim Lesen der MFT; wird auf ganze Records gerundet
const READ_CHUNK_SIZE: usize = 1024 * 1024;

/// Fortschritt alle N Records melden
const PROGRESS_INTERVAL: u64 = 16_384;

/// Der MFT-Reader - liest die Master File Table direkt aus
pub struct MftReader {
    /// Das Laufwerk (z.B. "C:")
    drive_letter: String,

    /// Bytes pro Sektor (normalerweise 512)
    bytes_per_sector: u32,

    /// Sektoren pro Cluster (variiert)
    sectors_per_cluster: u32,

    /// Bytes pro MFT-Record (normalerweise 1024)
    bytes_per_mft_record: u32,

    /// Start-Cluster der MFT
    mft_start_cluster: u64,

    /// Gesamtzahl der Cluster
    #[allow(dead_code)]
    total_clusters: u64,

    /// Bytes pro Cluster (berechnet)
    bytes_per_cluster: u64,
}

impl MftReader {
    /// Erstellt einen neuen MFT-Reader für das angegebene Laufwerk
    ///
    /// # Beispiel
    /// ```no_run
    /// use rustree::mft::MftReader;
    /// let reader = MftReader::new("C:")?;
    /// # Ok::<(), rustree::mft::MftError>(())
    /// ```
    pub fn new(drive: &str) -> Result<Self, MftError> {
        // Laufwerksbuchstabe normalisieren
        let drive_letter = drive.trim_end_matches('\\').to_uppercase();

        // Boot-Sektor lesen um NTFS-Parameter zu ermitteln
        let boot_sector = Self::read_boot_sector(&drive_letter)?;

        let bytes_per_cluster =
            boot_sector.bytes_per_sector as u64 * boot_sector.sectors_per_cluster as u64;

        Ok(Self {
            drive_letter,
            bytes_per_sector: boot_sector.bytes_per_sector,
            sectors_per_cluster: boot_sector.sectors_per_cluster,
            bytes_per_mft_record: boot_sector.bytes_per_mft_record,
            mft_start_cluster: boot_sector.mft_start_cluster,
            total_clusters: boot_sector.total_clusters,
            bytes_per_cluster,
        })
    }

    /// Gibt Informationen über das Laufwerk zurück (für Debugging)
    pub fn info(&self) -> String {
        format!(
            "Drive: {}\n\
             Bytes/Sector: {}\n\
             Sectors/Cluster: {}\n\
             Bytes/Cluster: {}\n\
             Bytes/MFT Record: {}\n\
             MFT Start Cluster: {}",
            self.drive_letter,
            self.bytes_per_sector,
            self.sectors_per_cluster,
            self.bytes_per_cluster,
            self.bytes_per_mft_record,
            self.mft_start_cluster
        )
    }

    /// Scannt die MFT und gibt alle Datei-Einträge zurück
    ///
    /// Dies ist die Hauptfunktion - sie liest die gesamte MFT
    /// und extrahiert alle Datei-/Ordner-Informationen.
    ///
    /// # Parameter
    /// - `progress_callback`: Wird mit Werten von 0.0 bis 1.0 aufgerufen
    ///
    /// # Rückgabe
    /// HashMap mit MFT-Referenz als Key und FileEntry als Value
    #[cfg(target_os = "windows")]
    pub fn scan<F>(&self, progress_callback: F) -> Result<HashMap<u64, FileEntry>, MftError>
    where
        F: Fn(f32, &str),
    {
        let record_size = self.bytes_per_mft_record as usize;
        if record_size == 0 || self.bytes_per_cluster == 0 {
            return Err(MftError::ReadError(
                "Boot-Sektor liefert keine Record- oder Cluster-Größe".to_string(),
            ));
        }

        let volume = Volume::open(&self.drive_letter)?;

        // Schritt 3: Record 0 beschreibt die MFT selbst
        progress_callback(0.0, "Lese $MFT-Record...");

        let mut mft_record = vec![0u8; record_size];
        let read = volume.read_at(self.mft_start_cluster * self.bytes_per_cluster, &mut mft_record)?;
        if read != record_size || !MftParser::apply_fixups(&mut mft_record) {
            return Err(MftError::InvalidRecord);
        }

        let data_attr = MftParser::attributes(&mft_record)
            .find(|(attr_type, attr)| {
                *attr_type == attribute_types::DATA && MftParser::attribute_name_length(attr) == 0
            })
            .map(|(_, attr)| attr)
            .ok_or(MftError::InvalidRecord)?;
        let runs = MftParser::parse_data_runs(data_attr).ok_or(MftError::InvalidRecord)?;
        let mft_size = MftParser::nonresident_real_size(data_attr).ok_or(MftError::InvalidRecord)?;
        let total_records = mft_size / record_size as u64;
        if total_records == 0 {
            return Err(MftError::InvalidRecord);
        }

        // Schritt 4: Fragmente nacheinander in großen Blöcken lesen. Die
        // Record-Nummer läuft über alle Fragmente hinweg durch - sie ist die
        // Position innerhalb der MFT-Datei, nicht auf der Platte.
        let chunk_records = (READ_CHUNK_SIZE / record_size).max(1);
        let mut buffer = vec![0u8; chunk_records * record_size];
        let mut entries: HashMap<u64, FileEntry> = HashMap::new();
        let mut mft_reference: u64 = 0;
        let mut next_progress = PROGRESS_INTERVAL;

        'runs: for run in &runs {
            let run_bytes = run.length * self.bytes_per_cluster;

            let Some(lcn) = run.lcn else {
                // Sparse: kein Platz auf der Platte belegt, die Records gelten als leer
                mft_reference += run_bytes / record_size as u64;
                continue;
            };

            let mut position = lcn * self.bytes_per_cluster;
            let mut remaining = run_bytes;

            while remaining > 0 && mft_reference < total_records {
                let wanted = remaining.min(buffer.len() as u64) as usize;
                let got = volume.read_at(position, &mut buffer[..wanted])?;
                let usable = got - got % record_size;
                if usable == 0 {
                    break 'runs; // Ende des Laufwerks oder Lesefehler
                }

                for record in buffer[..usable].chunks_exact_mut(record_size) {
                    if mft_reference >= total_records {
                        break 'runs;
                    }

                    if MftParser::apply_fixups(record) {
                        if let Some(entry) = MftParser::parse_record(record, mft_reference) {
                            // Systemdateien mit $ überspringen (optional)
                            if !entry.name.starts_with('$') || entry.name == "$Recycle.Bin" {
                                entries.insert(mft_reference, entry);
                            }
                        }
                    }

                    mft_reference += 1;
                    if mft_reference >= next_progress {
                        next_progress += PROGRESS_INTERVAL;
                        let progress = (mft_reference as f32 / total_records as f32).min(0.99);
                        let status = format!(
                            "{} von {} Records, {} Dateien/Ordner...",
                            mft_reference,
                            total_records,
                            entries.len()
                        );
                        progress_callback(progress, &status);
                    }
                }

                position += usable as u64;
                remaining -= usable as u64;
            }
        }

        let final_status = format!("{} Dateien/Ordner gefunden", entries.len());
        progress_callback(1.0, &final_status);

        Ok(entries)
    }

    #[cfg(not(target_os = "windows"))]
    pub fn scan<F>(&self, progress_callback: F) -> Result<HashMap<u64, FileEntry>, MftError>
    where
        F: Fn(f32, &str),
    {
        // Fallback für Nicht-Windows: Leere HashMap
        progress_callback(1.0, "Nicht unterstützt auf diesem OS");
        Ok(HashMap::new())
    }

    /// Liest den Boot-Sektor des NTFS-Volumes
    #[cfg(target_os = "windows")]
    fn read_boot_sector(drive: &str) -> Result<BootSectorInfo, MftError> {
        // Laufwerk öffnen (erfordert Admin-Rechte!)
        let volume = Volume::open(drive)?;

        // Boot-Sektor ist die ersten 512 Bytes
        let mut buffer = vec![0u8; 512];
        let read = volume.read_at(0, &mut buffer)?;
        if read < 512 {
            return Err(MftError::ReadError("Boot-Sektor unvollständig".to_string()));
        }

        // NTFS-Signatur prüfen (Bytes 3-7 sollten "NTFS" sein)
        if &buffer[3..7] != b"NTFS" {
            return Err(MftError::NotNtfs(drive.to_string()));
        }

        // Boot-Sektor parsen
        // Siehe: https://docs.microsoft.com/en-us/windows/win32/fileio/ntfs-technical-reference

        let bytes_per_sector = u16::from_le_bytes([buffer[11], buffer[12]]) as u32;

        // Sektoren pro Cluster: Werte über 128 sind als 2^(256 - n) kodiert
        // (große Cluster ab 128 KiB, seit Windows 10 1709)
        let sectors_per_cluster = match buffer[13] {
            0 => return Err(MftError::ReadError("Cluster-Größe 0".to_string())),
            n if n > 128 => 1u32 << (256 - n as u32),
            n => n as u32,
        };

        let mft_start_cluster = u64::from_le_bytes([
            buffer[48], buffer[49], buffer[50], buffer[51], buffer[52], buffer[53], buffer[54],
            buffer[55],
        ]);
        let total_sectors = u64::from_le_bytes([
            buffer[40], buffer[41], buffer[42], buffer[43], buffer[44], buffer[45], buffer[46],
            buffer[47],
        ]);

        // Bytes per MFT Record (kann negativ sein als Log2)
        let clusters_per_mft_record = buffer[64] as i8;
        let bytes_per_mft_record = if clusters_per_mft_record < 0 {
            1u32 << (-clusters_per_mft_record as u32)
        } else {
            (clusters_per_mft_record as u32) * sectors_per_cluster * bytes_per_sector
        };

        Ok(BootSectorInfo {
            bytes_per_sector,
            sectors_per_cluster,
            bytes_per_mft_record,
            mft_start_cluster,
            total_clusters: total_sectors / sectors_per_cluster as u64,
        })
    }

    #[cfg(not(target_os = "windows"))]
    fn read_boot_sector(_drive: &str) -> Result<BootSectorInfo, MftError> {
        // Fallback für Nicht-Windows (zum Kompilieren/Testen)
        Ok(BootSectorInfo {
            bytes_per_sector: 512,
            sectors_per_cluster: 8,
            bytes_per_mft_record: 1024,
            mft_start_cluster: 0,
            total_clusters: 0,
        })
    }
}

/// Informationen aus dem NTFS Boot-Sektor
struct BootSectorInfo {
    bytes_per_sector: u32,
    sectors_per_cluster: u32,
    bytes_per_mft_record: u32,
    mft_start_cluster: u64,
    total_clusters: u64,
}

/// Ein geöffnetes Raw-Volume; das Handle wird beim Drop geschlossen
#[cfg(target_os = "windows")]
struct Volume {
    handle: HANDLE,
}

#[cfg(target_os = "windows")]
impl Volume {
    /// Öffnet `\\.\<Laufwerk>` lesend (erfordert Admin-Rechte)
    fn open(drive_letter: &str) -> Result<Self, MftError> {
        let path: Vec<u16> = format!("\\\\.\\{}", drive_letter)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            )
        };

        match handle {
            Ok(handle) => Ok(Self { handle }),
            // HRESULT aus Win32-Fehlern: 0x8007xxxx, untere 16 Bit = Fehlercode
            Err(e) if e.code().0 as u32 == 0x8007_0005 => Err(MftError::AccessDenied),
            Err(e) if matches!(e.code().0 as u32, 0x8007_0002 | 0x8007_0003) => {
                Err(MftError::DriveNotFound(drive_letter.to_string()))
            }
            Err(e) => Err(MftError::WindowsError(e)),
        }
    }

    /// Liest an einer absoluten Byte-Position; Offset und Länge müssen
    /// Vielfache der Sektorgröße sein (Raw-Device)
    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> Result<usize, MftError> {
        let mut bytes_read = 0u32;
        unsafe {
            SetFilePointerEx(self.handle, offset as i64, None, FILE_BEGIN)?;
            ReadFile(self.handle, Some(buffer), Some(&mut bytes_read), None)?;
        }
        Ok(bytes_read as usize)
    }
}

#[cfg(target_os = "windows")]
impl Drop for Volume {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "windows")]
    fn test_boot_sector_reading() {
        // Dieser Test funktioniert nur als Admin
        match MftReader::new("C:") {
            Ok(reader) => {
                println!("{}", reader.info());
                assert!(reader.bytes_per_sector > 0);
                assert!(reader.bytes_per_mft_record > 0);
            }
            Err(MftError::AccessDenied) => {
                println!("Test übersprungen - keine Admin-Rechte");
            }
            Err(e) => {
                panic!("Unerwarteter Fehler: {:?}", e);
            }
        }
    }
}
