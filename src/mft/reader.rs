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
//!
//! # Warum ohne Cache lesen?
//!
//! Ein Raw-Volume liest sich über den Windows-Cache nur mit ein paar hundert
//! MB/s: jeder Block wird erst in den Cache kopiert und von dort in unseren
//! Puffer. Mit `FILE_FLAG_NO_BUFFERING` liefert die Platte direkt in unseren
//! Puffer, dafür müssen Puffer-Adresse, Offset und Länge Vielfache der
//! Sektorgröße sein ([`AlignedBuffer`]). Für eine MFT von mehreren GB ist
//! das der Unterschied zwischen zwanzig Sekunden und zwei.

use super::parser::{attribute_types, DataRun, ExtensionRecord, MftParser, ParsedRecord};
use super::types::{FileEntry, MftError};
use rayon::prelude::*;
use std::alloc::{alloc_zeroed, dealloc, handle_alloc_error, Layout};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::ptr::NonNull;

#[cfg(target_os = "windows")]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{CloseHandle, GENERIC_READ, HANDLE},
        Storage::FileSystem::{
            CreateFileW, ReadFile, SetFilePointerEx, FILE_ATTRIBUTE_NORMAL, FILE_BEGIN,
            FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_NO_BUFFERING, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        },
    },
};

/// Blockgröße beim Lesen der MFT; wird auf ganze Records gerundet. Groß
/// genug, dass ein Lesezugriff die Platte auslastet, klein genug für zwei
/// Puffer im Wechsel.
const READ_CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// Ausrichtung der Lesepuffer: deckt 512-Byte- und 4K-Sektoren ab
const BUFFER_ALIGNMENT: usize = 4096;

/// Obergrenze für die Record-Anzahl, ab der der Boot-Sektor als kaputt gilt
/// (eine MFT dieser Größe wäre ein Terabyte)
const MAX_RECORDS: usize = 1 << 30;

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
        // Laufwerksbuchstaben normalisieren
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
    /// Alle Dateien und Ordner, aufsteigend nach MFT-Referenz
    #[cfg(target_os = "windows")]
    pub fn scan<F>(&self, progress_callback: F) -> Result<Vec<FileEntry>, MftError>
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
    ) -> Result<Vec<FileEntry>, MftError>
    where
        F: Fn(f32, &str),
    {
        let record_size = self.bytes_per_mft_record as usize;
        let bytes_per_cluster = self.bytes_per_cluster as usize;
        if record_size == 0 || bytes_per_cluster == 0 {
            return Err(MftError::ReadError(
                "Boot-Sektor liefert keine Record- oder Cluster-Größe".to_string(),
            ));
        }

        let volume = Volume::open(&self.drive_letter, true)?;

        // Schritt 3: Record 0 beschreibt die MFT selbst. Ohne Cache muss
        // die Leselänge ein Vielfaches der Sektorgröße sein: ganze Cluster.
        progress_callback(0.0, "Lese $MFT-Record...");

        let mut first_cluster = AlignedBuffer::new(record_size.div_ceil(bytes_per_cluster) * bytes_per_cluster);
        let read = volume.read_at(
            self.mft_start_cluster * self.bytes_per_cluster,
            first_cluster.as_mut_slice(),
        )?;
        if read < record_size {
            return Err(MftError::InvalidRecord);
        }
        let mft_record = &mut first_cluster.as_mut_slice()[..record_size];
        if !MftParser::apply_fixups(mft_record) {
            return Err(MftError::InvalidRecord);
        }

        let data_attr = MftParser::attributes(mft_record)
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
    pub fn scan_dump<F>(path: &Path, progress_callback: F) -> Result<Vec<FileEntry>, MftError>
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
    pub fn scan<F>(&self, progress_callback: F) -> Result<Vec<FileEntry>, MftError>
    where
        F: Fn(f32, &str),
    {
        // Fallback für Nicht-Windows: keine Einträge
        progress_callback(1.0, "Nicht unterstützt auf diesem OS");
        Ok(Vec::new())
    }

    /// Liest den Boot-Sektor des NTFS-Volumes
    #[cfg(target_os = "windows")]
    fn read_boot_sector(drive: &str) -> Result<BootSectorInfo, MftError> {
        // Laufwerk öffnen (erfordert Admin-Rechte!); die 512 Bytes dürfen
        // durch den Cache gehen
        let volume = Volume::open(drive, false)?;

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

            // Run-Längen sind ganze Cluster, der Puffer ein Vielfaches davon:
            // beides bleibt sektorausgerichtet, wie es das Lesen ohne Cache verlangt
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
///
/// Die Einträge landen dicht in einem Vektor, Index = Record-Nummer. Das
/// spart das Hashen von Millionen Schlüsseln, und Erweiterungs-Records finden
/// ihren Basis-Record ohne Suche - egal ob sie vor oder nach ihm kommen.
fn scan_source<F>(
    source: &mut dyn RecordSource,
    record_size: usize,
    total_records: u64,
    mut dump: Option<&mut BufWriter<File>>,
    progress_callback: F,
) -> Result<Vec<FileEntry>, MftError>
where
    F: Fn(f32, &str),
{
    let slots = usize::try_from(total_records)
        .ok()
        .filter(|&n| n <= MAX_RECORDS)
        .ok_or_else(|| {
            MftError::ReadError(format!("MFT mit {} Records ist unplausibel groß", total_records))
        })?;

    let chunk_records = (READ_CHUNK_SIZE / record_size).max(1);
    let mut current = AlignedBuffer::new(chunk_records * record_size);
    let mut next = AlignedBuffer::new(chunk_records * record_size);

    let mut entries: Vec<Option<FileEntry>> = vec![None; slots];
    let mut name_ranks: Vec<u8> = vec![0; slots];
    let mut extensions: Vec<ExtensionRecord> = Vec::new();
    let mut found: usize = 0;
    let mut first_reference: u64 = 0;

    let mut usable = source.read_chunk(current.as_mut_slice())?;
    while usable > 0 && first_reference < total_records {
        // Rohdaten vor den Fixups sichern, damit der Dump der Platte entspricht
        if let Some(dump) = dump.as_mut() {
            dump.write_all(&current.as_slice()[..usable])?;
        }

        let block = &mut current.as_mut_slice()[..usable];
        let (next_usable, parsed) = rayon::join(
            || source.read_chunk(next.as_mut_slice()),
            || parse_block(block, record_size, first_reference, total_records),
        );

        for (reference, record) in parsed {
            match record {
                ParsedRecord::Entry { entry, name_rank } => {
                    let slot = reference as usize;
                    name_ranks[slot] = name_rank;
                    if entries[slot].replace(entry).is_none() {
                        found += 1;
                    }
                }
                ParsedRecord::Extension(extension) => extensions.push(extension),
            }
        }
        first_reference += (usable / record_size) as u64;

        let progress = (first_reference as f32 / total_records as f32).min(0.99);
        let status = format!(
            "{} von {} Records, {} Dateien/Ordner...",
            first_reference.min(total_records),
            total_records,
            found
        );
        progress_callback(progress, &status);

        usable = next_usable?;
        std::mem::swap(&mut current, &mut next);
    }

    if let Some(dump) = dump.as_mut() {
        dump.flush()?;
    }

    merge_extensions(&mut entries, &mut name_ranks, extensions);

    // Verdichten: nur belegte Slots, ohne Platzhalter, deren Name nie kam,
    // und ohne NTFS-Systemdateien ($MFT, $LogFile, ...)
    let entries: Vec<FileEntry> = entries
        .into_iter()
        .flatten()
        .filter(|entry| keep_entry(&entry.name))
        .collect();

    let final_status = format!("{} Dateien/Ordner gefunden", entries.len());
    progress_callback(1.0, &final_status);

    Ok(entries)
}

/// Trägt die ausgelagerten Attribute in die Basis-Records ein
///
/// Die Größe kommt aus dem Record mit dem ersten `$DATA`-Stück (nur dort
/// meldet der Parser sie), der Name gewinnt nach Rang - so wie innerhalb
/// eines Records auch.
fn merge_extensions(
    entries: &mut [Option<FileEntry>],
    name_ranks: &mut [u8],
    extensions: Vec<ExtensionRecord>,
) {
    for extension in extensions {
        let Ok(slot) = usize::try_from(extension.base_reference) else {
            continue;
        };
        let Some(Some(entry)) = entries.get_mut(slot) else {
            continue; // Basis-Record gelöscht oder unbekannt
        };
        if let Some(size) = extension.data_size {
            entry.size = size;
        }
        if let Some(name) = extension.file_name {
            if name.rank > name_ranks[slot] {
                name_ranks[slot] = name.rank;
                entry.name = name.name;
                entry.parent_reference = name.parent_reference;
            }
        }
    }
}

/// Systemdateien mit $ überspringen - bis auf den Papierkorb
fn keep_entry(name: &str) -> bool {
    !name.is_empty() && (!name.starts_with('$') || name == "$Recycle.Bin")
}

/// Parst einen Block Records parallel; Records jenseits von `total_records`
/// (Rest des letzten Fragments) werden ignoriert
fn parse_block(
    block: &mut [u8],
    record_size: usize,
    first_reference: u64,
    total_records: u64,
) -> Vec<(u64, ParsedRecord)> {
    block
        .par_chunks_mut(record_size)
        .enumerate()
        .filter_map(|(index, record)| {
            let mft_reference = first_reference + index as u64;
            if mft_reference >= total_records || !MftParser::apply_fixups(record) {
                return None;
            }
            MftParser::parse(record, mft_reference).map(|parsed| (mft_reference, parsed))
        })
        .collect()
}

/// Lesepuffer mit sektorausgerichteter Startadresse
///
/// `Vec<u8>` garantiert nur die Ausrichtung von `u8`; `FILE_FLAG_NO_BUFFERING`
/// verlangt Sektorgrenzen. Der Puffer wird einmal pro Scan angelegt.
struct AlignedBuffer {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedBuffer {
    fn new(len: usize) -> Self {
        let layout = Layout::from_size_align(len.max(1), BUFFER_ALIGNMENT)
            .expect("Puffergröße passt in den Adressraum");
        // SAFETY: das Layout hat eine Größe > 0
        let ptr = unsafe { alloc_zeroed(layout) };
        let Some(ptr) = NonNull::new(ptr) else {
            handle_alloc_error(layout);
        };
        Self { ptr, layout }
    }

    fn as_slice(&self) -> &[u8] {
        // SAFETY: der Speicher ist gültig, initialisiert und gehört uns
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.layout.size()) }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: wie as_slice, und &mut self garantiert exklusiven Zugriff
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }
}

impl Drop for AlignedBuffer {
    fn drop(&mut self) {
        // SAFETY: ptr stammt aus alloc_zeroed mit genau diesem Layout
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) }
    }
}

// SAFETY: ein eigener Heap-Block ohne geteilten Zustand; wie ein Vec<u8>
unsafe impl Send for AlignedBuffer {}

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
    ///
    /// `unbuffered` umgeht den Windows-Cache; dann müssen Puffer, Offsets
    /// und Längen sektorausgerichtet sein.
    fn open(drive_letter: &str, unbuffered: bool) -> Result<Self, MftError> {
        let path: Vec<u16> = format!("\\\\.\\{}", drive_letter)
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let mut flags: FILE_FLAGS_AND_ATTRIBUTES = FILE_ATTRIBUTE_NORMAL;
        if unbuffered {
            flags |= FILE_FLAG_NO_BUFFERING;
        }

        let handle = unsafe {
            CreateFileW(
                PCWSTR(path.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                flags,
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
    use crate::mft::parser::test_support::*;
    use crate::mft::parser::file_name_namespace;

    /// Records aus dem Speicher, als wären sie eine MFT
    struct SliceSource {
        data: Vec<u8>,
        position: usize,
    }

    impl RecordSource for SliceSource {
        fn read_chunk(&mut self, buffer: &mut [u8]) -> Result<usize, MftError> {
            let count = (self.data.len() - self.position).min(buffer.len());
            buffer[..count].copy_from_slice(&self.data[self.position..self.position + count]);
            self.position += count;
            Ok(count)
        }
    }

    fn scan_records(records: &[(u64, Vec<u8>)], total: u64) -> Vec<FileEntry> {
        let mut data = vec![0u8; total as usize * 1024];
        for (reference, record) in records {
            let start = *reference as usize * 1024;
            data[start..start + 1024].copy_from_slice(record);
        }
        let mut source = SliceSource { data, position: 0 };
        scan_source(&mut source, 1024, total, None, |_, _| {}).unwrap()
    }

    #[test]
    fn merges_extension_records_into_base() {
        // Record 6: Basis mit 8.3-Namen und $ATTRIBUTE_LIST, $DATA ausgelagert.
        // Record 3 (vor der Basis!) und 7: Erweiterungen mit langem Namen,
        // dem ersten $DATA-Stück (Größe zählt) und einem zweiten (zählt nicht).
        let root = build_record(0x03, &[file_name_attr(5, ".", file_name_namespace::WIN32)]);
        let base = build_record(
            0x01,
            &[
                resident_attr(attribute_types::ATTRIBUTE_LIST, 0, &[0u8; 32]),
                file_name_attr(5, "ONEDRI~1.KG", file_name_namespace::DOS),
            ],
        );
        let mut first_piece = build_record(
            0x01,
            &[
                file_name_attr(5, "OneDrive.kg", file_name_namespace::WIN32),
                nonresident_attr(attribute_types::DATA, 5_000, &[0x11, 0x01, 0x20, 0x00]),
            ],
        );
        make_extension(&mut first_piece, 6);
        let mut later_piece = nonresident_attr(attribute_types::DATA, 5_000, &[0x11, 0x01, 0x30, 0x00]);
        later_piece[16..24].copy_from_slice(&8u64.to_le_bytes());
        let mut second_piece = build_record(0x01, &[later_piece]);
        make_extension(&mut second_piece, 6);

        let entries = scan_records(&[(5, root), (6, base), (3, first_piece), (7, second_piece)], 8);

        assert_eq!(entries.len(), 2);
        let file = entries.iter().find(|e| e.mft_reference == 6).unwrap();
        assert_eq!(file.name, "OneDrive.kg");
        assert_eq!(file.parent_reference, 5);
        assert_eq!(file.size, 5_000);
        assert_eq!(entries[0].mft_reference, 5);
    }

    #[test]
    fn drops_placeholders_and_system_files() {
        let root = build_record(0x03, &[file_name_attr(5, ".", file_name_namespace::WIN32)]);
        let nameless = build_record(
            0x01,
            &[resident_attr(attribute_types::ATTRIBUTE_LIST, 0, &[0u8; 32])],
        );
        let system = build_record(0x01, &[file_name_attr(5, "$LogFile", file_name_namespace::WIN32)]);
        let recycle = build_record(0x03, &[file_name_attr(5, "$Recycle.Bin", file_name_namespace::WIN32)]);

        let entries = scan_records(&[(5, root), (6, nameless), (2, system), (9, recycle)], 10);
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec![".", "$Recycle.Bin"]);
    }

    #[test]
    fn aligned_buffer_is_sector_aligned() {
        let mut buffer = AlignedBuffer::new(3 * 1024);
        assert_eq!(buffer.as_slice().len(), 3 * 1024);
        assert_eq!(buffer.as_mut_slice().as_ptr() as usize % BUFFER_ALIGNMENT, 0);
        buffer.as_mut_slice()[3071] = 7;
        assert_eq!(buffer.as_slice()[3071], 7);
    }

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
