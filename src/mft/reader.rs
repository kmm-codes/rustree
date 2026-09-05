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
//! 2. Boot-Sektor lesen (erste 512 Bytes)
//! 3. Aus dem Boot-Sektor die MFT-Position ermitteln
//! 4. MFT-Records sequentiell lesen

use super::parser::MftParser;
use super::types::{FileEntry, MftError};
use std::collections::HashMap;

#[cfg(target_os = "windows")]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{HANDLE, CloseHandle, GENERIC_READ},
        Storage::FileSystem::{
            CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE,
            OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL,
            ReadFile, SetFilePointerEx, FILE_BEGIN,
        },
    },
};

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
        let mut entries = HashMap::new();

        // Pfad zum Raw-Device
        let path: Vec<u16> = format!("\\\\.\\{}", self.drive_letter)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // Laufwerk öffnen
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            )?
        };

        // MFT-Position berechnen
        let mft_offset = self.mft_start_cluster * self.bytes_per_cluster;

        progress_callback(0.0, "Suche MFT...");

        // Zur MFT seekenn
        let mut new_pos = 0i64;
        unsafe {
            SetFilePointerEx(handle, mft_offset as i64, Some(&mut new_pos), FILE_BEGIN)?;
        }

        // Buffer für einen MFT-Record
        let record_size = self.bytes_per_mft_record as usize;
        let mut record_buffer = vec![0u8; record_size];

        // Wir lesen in Batches für bessere Performance
        // Die MFT-Größe ist unbekannt, wir lesen bis wir ungültige Records finden
        let mut mft_reference: u64 = 0;
        let mut valid_records = 0u64;
        let mut invalid_count = 0;
        let max_invalid = 100; // Nach 100 ungültigen Records stoppen

        // Schätzung für Progress (wird beim Lesen angepasst)
        let estimated_records = 1_000_000u64; // Typische Größe

        progress_callback(0.01, "Lese MFT-Records...");

        loop {
            // Record lesen
            let mut bytes_read = 0u32;
            let read_result = unsafe {
                ReadFile(
                    handle,
                    Some(&mut record_buffer),
                    Some(&mut bytes_read),
                    None,
                )
            };

            // Lesefehler oder EOF
            if read_result.is_err() || bytes_read == 0 {
                break;
            }

            // Record parsen
            if let Some(entry) = MftParser::parse_record(&record_buffer, mft_reference) {
                // Systemdateien mit $ überspringen (optional)
                if !entry.name.starts_with('$') || entry.name == "$Recycle.Bin" {
                    entries.insert(entry.mft_reference, entry);
                }
                valid_records += 1;
                invalid_count = 0; // Reset bei gültigem Record
            } else {
                // Ungültiger Record
                invalid_count += 1;
                if invalid_count >= max_invalid {
                    // Wahrscheinlich Ende der MFT erreicht
                    break;
                }
            }

            mft_reference += 1;

            // Progress alle 10000 Records aktualisieren
            if mft_reference % 10000 == 0 {
                let progress = (mft_reference as f32 / estimated_records as f32).min(0.99);
                let status = format!("{} Dateien gefunden...", valid_records);
                progress_callback(progress, &status);
            }

            // Safety limit: Nach 10 Millionen Records aufhören
            if mft_reference > 10_000_000 {
                break;
            }
        }

        // Handle schließen
        unsafe { CloseHandle(handle)?; }

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
        // Pfad zum Raw-Device erstellen: \\.\C:
        let path: Vec<u16> = format!("\\\\.\\{}", drive)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // Laufwerk öffnen (erfordert Admin-Rechte!)
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

        let handle = match handle {
            Ok(h) => h,
            Err(e) => {
                // Prüfe ob es ein Zugriffsfehler ist
                if e.code().0 as u32 == 0x80070005 {
                    return Err(MftError::AccessDenied);
                }
                return Err(MftError::WindowsError(e));
            }
        };

        // Boot-Sektor ist die ersten 512 Bytes
        let mut buffer = vec![0u8; 512];
        let mut bytes_read = 0u32;

        let read_result = unsafe {
            ReadFile(
                handle,
                Some(&mut buffer),
                Some(&mut bytes_read),
                None,
            )
        };

        // Handle schließen
        let _ = unsafe { CloseHandle(handle) };

        read_result?;

        // NTFS-Signatur prüfen (Bytes 3-7 sollten "NTFS" sein)
        if &buffer[3..7] != b"NTFS" {
            return Err(MftError::NotNtfs(drive.to_string()));
        }

        // Boot-Sektor parsen
        // Siehe: https://docs.microsoft.com/en-us/windows/win32/fileio/ntfs-technical-reference

        let bytes_per_sector = u16::from_le_bytes([buffer[11], buffer[12]]) as u32;
        let sectors_per_cluster = buffer[13] as u32;
        let mft_start_cluster = u64::from_le_bytes([
            buffer[48], buffer[49], buffer[50], buffer[51],
            buffer[52], buffer[53], buffer[54], buffer[55],
        ]);
        let total_sectors = u64::from_le_bytes([
            buffer[40], buffer[41], buffer[42], buffer[43],
            buffer[44], buffer[45], buffer[46], buffer[47],
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
