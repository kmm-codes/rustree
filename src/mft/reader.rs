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

use super::parser::{attribute_types, DataRun, MftParser};
use super::types::{FileEntry, MftError};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;

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

/// Blockgröße beim Lesen der MFT; wird auf ganze Records gerundet. Groß
/// genug, dass ein Lesezugriff die Platte auslastet, klein genug für zwei
/// Puffer im Wechsel.
const READ_CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// Kopf einer MFT-Dump-Datei: Magic (8), Record-Größe (u32), reserviert (u32)
const DUMP_MAGIC: &[u8; 8] = b"RTMFTv1\0";
const DUMP_HEADER_SIZE: usize = 16;

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
        self.scan_with_dump(None, progress_callback)
    }

    /// Wie [`MftReader::scan`], schreibt die rohen Records zusätzlich in eine
    /// Dump-Datei, die [`MftReader::scan_dump`] später ohne Admin-Rechte
    /// wieder einlesen kann - zum Messen und Testen des Parsers.
    #[cfg(target_os = "windows")]
    pub fn scan_with_dump<F>(
        &self,
        dump_to: Option<&Path>,
        progress_callback: F,
    ) -> Result<HashMap<u64, FileEntry>, MftError>
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

        let mut dump = match dump_to {
            Some(path) => Some(Self::create_dump(path, record_size)?),
            None => None,
        };

        // Schritt 4: Fragmente nacheinander in großen Blöcken lesen
        let mut source = VolumeSource::new(&volume, runs, self.bytes_per_cluster, record_size);
        scan_source(&mut source, record_size, total_records, dump.as_mut(), progress_callback)
    }

    /// Liest eine Dump-Datei aus [`MftReader::scan_with_dump`] statt des
    /// Laufwerks - braucht keine Admin-Rechte
    pub fn scan_dump<F>(path: &Path, progress_callback: F) -> Result<HashMap<u64, FileEntry>, MftError>
    where
        F: Fn(f32, &str),
    {
        let mut file = File::open(path)?;
        let mut header = [0u8; DUMP_HEADER_SIZE];
        file.read_exact(&mut header)?;
        if &header[0..8] != DUMP_MAGIC {
            return Err(MftError::ReadError(format!(
                "{} ist keine MFT-Dump-Datei",
                path.display()
            )));
        }
        let record_size = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        if record_size == 0 {
            return Err(MftError::ReadError("Dump ohne Record-Größe".to_string()));
        }
        let total_records = (file.metadata()?.len().saturating_sub(DUMP_HEADER_SIZE as u64))
            / record_size as u64;

        let mut source = FileSource { file, record_size };
        scan_source(&mut source, record_size, total_records, None, progress_callback)
    }

    /// Legt die Dump-Datei an und schreibt den Kopf
    fn create_dump(path: &Path, record_size: usize) -> Result<BufWriter<File>, MftError> {
        let mut writer = BufWriter::with_capacity(READ_CHUNK_SIZE, File::create(path)?);
        let mut header = [0u8; DUMP_HEADER_SIZE];
        header[0..8].copy_from_slice(DUMP_MAGIC);
        header[8..12].copy_from_slice(&(record_size as u32).to_le_bytes());
        writer.write_all(&header)?;
        Ok(writer)
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

/// Woher die MFT-Records kommen: vom Laufwerk oder aus einer Dump-Datei
///
/// Liefert die MFT als fortlaufenden Bytestrom in Blöcken aus ganzen
/// Records, unabhängig davon, wie sie auf der Platte verteilt ist.
/// `Send`, weil der nächste Block auf einem anderen Thread gelesen wird,
/// während der aktuelle geparst wird.
trait RecordSource: Send {
    /// Füllt den Puffer mit dem nächsten Block; 0 bedeutet Ende
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, MftError>;
}

/// Liest die Fragmente der MFT vom Raw-Volume
#[cfg(target_os = "windows")]
struct VolumeSource<'a> {
    volume: &'a Volume,
    runs: Vec<DataRun>,
    bytes_per_cluster: u64,
    record_size: usize,
    next_run: usize,
    /// Nächste Leseposition auf der Platte (ungenutzt bei sparse Runs)
    position: u64,
    /// Verbleibende Bytes im aktuellen Run
    remaining: u64,
    sparse: bool,
}

#[cfg(target_os = "windows")]
impl<'a> VolumeSource<'a> {
    fn new(volume: &'a Volume, runs: Vec<DataRun>, bytes_per_cluster: u64, record_size: usize) -> Self {
        Self {
            volume,
            runs,
            bytes_per_cluster,
            record_size,
            next_run: 0,
            position: 0,
            remaining: 0,
            sparse: false,
        }
    }
}

#[cfg(target_os = "windows")]
impl RecordSource for VolumeSource<'_> {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, MftError> {
        loop {
            if self.remaining == 0 {
                // Nächstes Fragment
                let Some(run) = self.runs.get(self.next_run) else {
                    return Ok(0);
                };
                self.next_run += 1;
                self.remaining = run.length * self.bytes_per_cluster;
                match run.lcn {
                    Some(lcn) => {
                        self.sparse = false;
                        self.position = lcn * self.bytes_per_cluster;
                    }
                    None => self.sparse = true,
                }
                continue;
            }

            let wanted = self.remaining.min(buffer.len() as u64) as usize;
            if self.sparse {
                // Kein Platz auf der Platte belegt: die Records gelten als leer
                buffer[..wanted].fill(0);
                self.remaining -= wanted as u64;
                return Ok(wanted);
            }

            let got = self.volume.read_at(self.position, &mut buffer[..wanted])?;
            let usable = got - got % self.record_size;
            if usable == 0 {
                return Ok(0); // Ende des Laufwerks oder Lesefehler
            }
            self.position += usable as u64;
            self.remaining -= usable as u64;
            return Ok(usable);
        }
    }
}

/// Liest eine Dump-Datei sequentiell
struct FileSource {
    file: File,
    record_size: usize,
}

impl RecordSource for FileSource {
    fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, MftError> {
        let mut filled = 0;
        while filled < buffer.len() {
            let got = self.file.read(&mut buffer[filled..])?;
            if got == 0 {
                break;
            }
            filled += got;
        }
        Ok(filled - filled % self.record_size)
    }
}

/// Der eigentliche Scan: Records blockweise holen, Fixups anwenden, parsen
///
/// Zwei Puffer im Wechsel: während ein Block geparst wird (parallel über
/// alle Kerne, jeder Record ist unabhängig), liest ein zweiter Thread schon
/// den nächsten von der Platte. Die Record-Nummer läuft über alle Blöcke
/// hinweg durch - sie ist die Position innerhalb der MFT, nicht auf der Platte.
fn scan_source<F>(
    source: &mut dyn RecordSource,
    record_size: usize,
    total_records: u64,
    mut dump: Option<&mut BufWriter<File>>,
    progress_callback: F,
) -> Result<HashMap<u64, FileEntry>, MftError>
where
    F: Fn(f32, &str),
{
    let chunk_records = (READ_CHUNK_SIZE / record_size).max(1);
    let mut current = vec![0u8; chunk_records * record_size];
    let mut next = vec![0u8; chunk_records * record_size];

    // Fast jeder Record ist eine Datei oder ein Ordner; einmal reservieren
    // erspart ein Dutzend Rehashes von Millionen Einträgen.
    let mut entries: HashMap<u64, FileEntry> =
        HashMap::with_capacity(total_records.min(50_000_000) as usize);
    let mut first_reference: u64 = 0;

    let mut usable = source.read_chunk(&mut current)?;
    while usable > 0 && first_reference < total_records {
        // Rohdaten vor den Fixups sichern, damit der Dump der Platte entspricht
        if let Some(dump) = dump.as_mut() {
            dump.write_all(&current[..usable])?;
        }

        let block = &mut current[..usable];
        let (next_usable, parsed) = rayon::join(
            || source.read_chunk(&mut next),
            || parse_block(block, record_size, first_reference, total_records),
        );

        entries.extend(parsed);
        first_reference += (usable / record_size) as u64;

        let progress = (first_reference as f32 / total_records as f32).min(0.99);
        let status = format!(
            "{} von {} Records, {} Dateien/Ordner...",
            first_reference.min(total_records),
            total_records,
            entries.len()
        );
        progress_callback(progress, &status);

        usable = next_usable?;
        std::mem::swap(&mut current, &mut next);
    }

    if let Some(dump) = dump.as_mut() {
        dump.flush()?;
    }

    let final_status = format!("{} Dateien/Ordner gefunden", entries.len());
    progress_callback(1.0, &final_status);

    Ok(entries)
}

/// Parst einen Block Records parallel; Records jenseits von `total_records`
/// (Rest des letzten Fragments) werden ignoriert
fn parse_block(
    block: &mut [u8],
    record_size: usize,
    first_reference: u64,
    total_records: u64,
) -> Vec<(u64, FileEntry)> {
    block
        .par_chunks_mut(record_size)
        .enumerate()
        .filter_map(|(index, record)| {
            let mft_reference = first_reference + index as u64;
            if mft_reference >= total_records || !MftParser::apply_fixups(record) {
                return None;
            }
            let entry = MftParser::parse_record(record, mft_reference)?;
            // Systemdateien mit $ überspringen (optional)
            if entry.name.starts_with('$') && entry.name != "$Recycle.Bin" {
                return None;
            }
            Some((mft_reference, entry))
        })
        .collect()
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

// SAFETY: ein Datei-Handle darf von jedem Thread benutzt werden; wir lesen
// nur, und immer von genau einem Thread zur Zeit.
#[cfg(target_os = "windows")]
unsafe impl Send for Volume {}
#[cfg(target_os = "windows")]
unsafe impl Sync for Volume {}

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
