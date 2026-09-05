//! MFT Record Parser
//!
//! # Struktur eines MFT-Records
//!
//! Jeder MFT-Record ist normalerweise 1024 Bytes groß und hat folgende Struktur:
//!
//! ```text
//! +-------------------+
//! | FILE Header       |  <- Signatur "FILE" (4 Bytes)
//! | (48 Bytes)        |
//! +-------------------+
//! | Attribute 1       |  <- z.B. $STANDARD_INFORMATION
//! +-------------------+
//! | Attribute 2       |  <- z.B. $FILE_NAME
//! +-------------------+
//! | Attribute 3       |  <- z.B. $DATA
//! +-------------------+
//! | ...               |
//! +-------------------+
//! | End Marker        |  <- 0xFFFFFFFF
//! +-------------------+
//! ```
//!
//! # Fixups
//!
//! NTFS ersetzt die letzten zwei Bytes jedes Sektors eines Records durch die
//! Update Sequence Number (USN); die Originalbytes stehen im Update Sequence
//! Array im Header. So erkennt NTFS halb geschriebene Records. Vor dem Parsen
//! müssen diese Bytes zurückgetauscht werden ([`MftParser::apply_fixups`]),
//! sonst sind pro Sektor zwei Bytes falsch - je nach Lage mitten in einem
//! Attribut.
//!
//! # Wichtige Attribute
//!
//! - `$STANDARD_INFORMATION` (0x10): Timestamps, Flags
//! - `$FILE_NAME` (0x30): Dateiname und Parent-Referenz, oft mehrfach
//!   (kurzer 8.3-Name und langer Name, Hardlinks)
//! - `$DATA` (0x80): Dateiinhalt oder Größeninformation; benannte
//!   `$DATA`-Attribute sind Alternate Data Streams
//!
//! # Data Runs
//!
//! Passt der Inhalt eines Attributs nicht in den Record (non-resident), steht
//! dort nur eine Liste von Fragmenten auf der Platte, die Data Runs. Für die
//! `$MFT` selbst brauchen wir sie, um alle Records zu finden: die MFT ist auf
//! realen Laufwerken fast immer fragmentiert.
//!
//! # Erweiterungs-Records
//!
//! Passen die Attribute selbst nicht mehr in einen Record - stark
//! fragmentierte Dateien mit langen Run-Listen, Dateien mit vielen Hardlinks
//! (jeder ist ein `$FILE_NAME`) - lagert NTFS sie in weitere Records aus und
//! verzeichnet das im `$ATTRIBUTE_LIST` (0x20) des Basis-Records. Die
//! ausgelagerten Records zeigen bei Offset 32 auf ihren Basis-Record. Ohne
//! sie fehlen genau die größten Dateien im Ergebnis, und manche Ordner
//! tragen nur ihren 8.3-Namen. [`MftParser::parse`] liefert sie deshalb als
//! [`ParsedRecord::Extension`] zum späteren Zusammenführen.

use super::types::FileEntry;

/// Attribut-Typen in einem MFT-Record
pub mod attribute_types {
    pub const STANDARD_INFORMATION: u32 = 0x10;
    pub const ATTRIBUTE_LIST: u32 = 0x20;
    pub const FILE_NAME: u32 = 0x30;
    pub const OBJECT_ID: u32 = 0x40;
    pub const VOLUME_NAME: u32 = 0x60;
    pub const VOLUME_INFORMATION: u32 = 0x70;
    pub const DATA: u32 = 0x80;
    pub const INDEX_ROOT: u32 = 0x90;
    pub const INDEX_ALLOCATION: u32 = 0xA0;
    pub const BITMAP: u32 = 0xB0;
    pub const END_MARKER: u32 = 0xFFFFFFFF;
}

/// Flags für Dateien
pub mod file_flags {
    pub const READ_ONLY: u32 = 0x0001;
    pub const HIDDEN: u32 = 0x0002;
    pub const SYSTEM: u32 = 0x0004;
    pub const DIRECTORY: u32 = 0x10000000;
}

/// Namespace eines `$FILE_NAME`-Attributs (Byte 65 im Inhalt)
pub mod file_name_namespace {
    /// Beliebige Unicode-Zeichen, case-sensitiv
    pub const POSIX: u8 = 0;
    /// Der lange Windows-Name
    pub const WIN32: u8 = 1;
    /// Der kurze 8.3-Name (PROGRA~1)
    pub const DOS: u8 = 2;
    /// Kurzer und langer Name sind identisch
    pub const WIN32_AND_DOS: u8 = 3;
}

/// Ein Fragment eines non-resident Attributs auf der Platte
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataRun {
    /// Erster Cluster; `None` bei einem sparse Run (nicht belegt, liest sich als Nullen)
    pub lcn: Option<u64>,
    /// Länge in Clustern
    pub length: u64,
}

/// Ergebnis von [`MftParser::parse`]
#[derive(Debug, Clone)]
pub enum ParsedRecord {
    /// Ein Basis-Record: eine Datei oder ein Ordner
    Entry {
        entry: FileEntry,
        /// Rang des gefundenen Namens (siehe [`name_rank`]); 0 = noch keiner
        name_rank: u8,
    },
    /// Ein Erweiterungs-Record mit Attributen, die zu einem anderen Record gehören
    Extension(ExtensionRecord),
}

/// Attribute aus einem Erweiterungs-Record, die der Basis-Record braucht
#[derive(Debug, Clone)]
pub struct ExtensionRecord {
    /// MFT-Referenz des Basis-Records (ohne Sequenznummer)
    pub base_reference: u64,
    /// Größe des unbenannten `$DATA`-Streams, falls sein erstes Stück hier liegt
    pub data_size: Option<u64>,
    /// Bester `$FILE_NAME` in diesem Record
    pub file_name: Option<ExtensionName>,
}

/// Ein Name aus einem Erweiterungs-Record
#[derive(Debug, Clone)]
pub struct ExtensionName {
    pub rank: u8,
    pub parent_reference: u64,
    pub name: String,
}

/// Der MFT-Parser verarbeitet rohe MFT-Records
pub struct MftParser;

impl MftParser {
    /// Wendet die Fixups (Update Sequence Array) auf einen Record an
    ///
    /// Gibt `false` zurück, wenn der Record inkonsistent ist: falsche
    /// Signatur, unplausibler Header oder ein Sektorende, dessen USN nicht
    /// passt (abgebrochener Schreibvorgang). So ein Record wird übersprungen.
    pub fn apply_fixups(record: &mut [u8]) -> bool {
        if record.len() < 8 || &record[0..4] != b"FILE" {
            return false;
        }

        let usa_offset = u16::from_le_bytes([record[4], record[5]]) as usize;
        let usa_count = u16::from_le_bytes([record[6], record[7]]) as usize;

        // Eintrag 0 ist die USN, danach je ein Originalwert pro Sektor
        if usa_count < 2 || usa_offset + usa_count * 2 > record.len() {
            return false;
        }
        let sectors = usa_count - 1;
        if record.len() % sectors != 0 {
            return false;
        }
        let sector_size = record.len() / sectors;
        if sector_size < 2 {
            return false;
        }

        let usn = [record[usa_offset], record[usa_offset + 1]];
        for sector in 0..sectors {
            let end = (sector + 1) * sector_size;
            if record[end - 2..end] != usn {
                return false;
            }
            let original = usa_offset + 2 + sector * 2;
            record[end - 2] = record[original];
            record[end - 1] = record[original + 1];
        }
        true
    }

    /// Iteriert über die Attribute eines (fixup-korrigierten) Records
    ///
    /// Liefert je Attribut den Typ und die kompletten Bytes inklusive Header.
    /// Bricht am End Marker oder bei einer unplausiblen Länge ab.
    pub fn attributes(record: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
        let first_attr_offset = if record.len() >= 24 && &record[0..4] == b"FILE" {
            u16::from_le_bytes([record[20], record[21]]) as usize
        } else {
            record.len()
        };
        AttributeIter {
            record,
            offset: first_attr_offset,
        }
    }

    /// Länge des Attributnamens in Zeichen (Offset 9); 0 = unbenannt
    pub fn attribute_name_length(attr: &[u8]) -> usize {
        attr.get(9).copied().unwrap_or(0) as usize
    }

    /// Parst einen rohen MFT-Record und extrahiert die Dateiinformationen
    ///
    /// Bequeme Variante von [`MftParser::parse`] für Basis-Records mit Namen:
    /// `None` bei leeren/gelöschten Records, Erweiterungs-Records und
    /// Records ohne Namen.
    pub fn parse_record(record: &[u8], mft_reference: u64) -> Option<FileEntry> {
        match Self::parse(record, mft_reference)? {
            ParsedRecord::Entry { entry, .. } if !entry.name.is_empty() => Some(entry),
            _ => None,
        }
    }

    /// Parst einen rohen MFT-Record
    ///
    /// # Parameter
    /// - `record`: Die Bytes des MFT-Records nach [`MftParser::apply_fixups`]
    /// - `mft_reference`: Die MFT-Referenznummer dieses Records
    ///
    /// # Rückgabe
    /// - [`ParsedRecord::Entry`] für eine Datei oder einen Ordner. Der Name
    ///   ist leer, wenn er per `$ATTRIBUTE_LIST` in einem Erweiterungs-Record
    ///   liegt; solche Einträge sind erst nach dem Zusammenführen brauchbar.
    /// - [`ParsedRecord::Extension`] für Records, die zu einer anderen Datei
    ///   gehören
    /// - `None` bei leeren/gelöschten Records und namenlosen Systemdaten
    pub fn parse(record: &[u8], mft_reference: u64) -> Option<ParsedRecord> {
        // Prüfe Signatur "FILE"
        if record.len() < 48 || &record[0..4] != b"FILE" {
            return None;
        }

        // Flags prüfen (Offset 22-23)
        let flags = u16::from_le_bytes([record[22], record[23]]);

        // Bit 0: Record in use (1 = in Benutzung)
        if flags & 0x01 == 0 {
            return None; // Gelöschter Record
        }

        let is_directory = flags & 0x02 != 0;

        // Offset 32: Referenz auf den Basis-Record, 0 bei Basis-Records
        // selbst. Die oberen 16 Bit sind die Sequenznummer.
        let base_record =
            u64::from_le_bytes(record[32..40].try_into().ok()?) & 0x0000_FFFF_FFFF_FFFF;

        // Attribute durchlaufen
        let mut best_name: Option<(u8, FileNameAttr)> = None;
        let mut size: Option<u64> = None;
        let mut is_hidden = false;
        let mut is_system = false;
        let mut has_attribute_list = false;

        for (attr_type, attr) in Self::attributes(record) {
            match attr_type {
                attribute_types::STANDARD_INFORMATION => {
                    // $STANDARD_INFORMATION enthält Flags
                    if let Some(std_info) = Self::parse_standard_information(attr) {
                        is_hidden = std_info.is_hidden;
                        is_system = std_info.is_system;
                    }
                }
                attribute_types::ATTRIBUTE_LIST => has_attribute_list = true,
                attribute_types::FILE_NAME => {
                    // $FILE_NAME enthält Name und Parent-Referenz. Der lange
                    // Windows-Name gewinnt gegen den 8.3-Namen (PROGRA~1),
                    // egal in welcher Reihenfolge sie im Record stehen.
                    if let Some(file_name) = Self::parse_file_name(attr) {
                        let rank = name_rank(file_name.namespace);
                        if best_name.as_ref().is_none_or(|(best, _)| rank > *best) {
                            best_name = Some((rank, file_name));
                        }
                    }
                }
                attribute_types::DATA => {
                    // Nur der unbenannte $DATA-Stream ist der Dateiinhalt;
                    // benannte Streams (z.B. Zone.Identifier) zählen nicht.
                    if Self::attribute_name_length(attr) == 0 {
                        if let Some(data_size) = Self::parse_data_attribute(attr) {
                            size = Some(data_size);
                        }
                    }
                }
                _ => {}
            }
        }

        if base_record != 0 {
            return Some(ParsedRecord::Extension(ExtensionRecord {
                base_reference: base_record,
                data_size: size,
                file_name: best_name.map(|(rank, file_name)| ExtensionName {
                    rank,
                    parent_reference: file_name.parent_reference,
                    name: file_name.name,
                }),
            }));
        }

        // Records ohne Namen sind Systemdaten - außer der Name liegt per
        // $ATTRIBUTE_LIST in einem Erweiterungs-Record: dann ein Platzhalter,
        // den das Zusammenführen füllt
        let (name_rank, parent_reference, name) = match best_name {
            Some((rank, file_name)) => (rank, file_name.parent_reference, file_name.name),
            None if has_attribute_list => (0, 0, String::new()),
            None => return None,
        };

        Some(ParsedRecord::Entry {
            entry: FileEntry {
                mft_reference,
                parent_reference,
                name,
                size: size.unwrap_or(0),
                is_directory,
                is_hidden,
                is_system,
            },
            name_rank,
        })
    }

    /// Parst das $STANDARD_INFORMATION Attribut
    fn parse_standard_information(attr: &[u8]) -> Option<StandardInfo> {
        // Non-resident flag (Offset 8)
        let non_resident = attr.get(8)? != &0;

        if non_resident {
            return None; // $STANDARD_INFORMATION ist immer resident
        }

        // Content offset (Offset 20-21)
        let content_offset = u16::from_le_bytes([*attr.get(20)?, *attr.get(21)?]) as usize;

        if content_offset + 48 > attr.len() {
            return None;
        }

        // DOS-Flags sind bei Offset 32 im Content
        let flags = u32::from_le_bytes([
            *attr.get(content_offset + 32)?,
            *attr.get(content_offset + 33)?,
            *attr.get(content_offset + 34)?,
            *attr.get(content_offset + 35)?,
        ]);

        Some(StandardInfo {
            is_hidden: flags & file_flags::HIDDEN != 0,
            is_system: flags & file_flags::SYSTEM != 0,
        })
    }

    /// Parst das $FILE_NAME Attribut
    fn parse_file_name(attr: &[u8]) -> Option<FileNameAttr> {
        // Non-resident check
        if attr.get(8)? != &0 {
            return None; // $FILE_NAME ist immer resident
        }

        // Content offset
        let content_offset = u16::from_le_bytes([*attr.get(20)?, *attr.get(21)?]) as usize;

        if content_offset + 66 > attr.len() {
            return None;
        }

        let content = &attr[content_offset..];

        // Parent-Referenz: untere 6 Bytes sind die Record-Nummer, die oberen
        // 2 Bytes die Sequenznummer (Wiederverwendungszähler)
        let parent_reference = u64::from_le_bytes([
            content[0], content[1], content[2], content[3], content[4], content[5], 0, 0,
        ]);

        // Namespace (Offset 65)
        let namespace = *content.get(65)?;

        // Namenlänge in Characters (Offset 64)
        let name_length = *content.get(64)? as usize;

        // Name startet bei Offset 66 (UTF-16LE). Direkt in den String
        // dekodieren: eine Allokation pro Name statt zwei - bei Millionen
        // Namen der teuerste Teil des Parsens.
        let name_bytes = content.get(66..66 + name_length * 2)?;
        let units = name_bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]));
        let mut name = String::with_capacity(name_length);
        name.extend(char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)));

        Some(FileNameAttr {
            parent_reference,
            name,
            namespace,
        })
    }

    /// Parst das $DATA Attribut um die Dateigröße zu ermitteln
    ///
    /// Bei stark fragmentierten Dateien verteilt NTFS die Run-Liste auf
    /// mehrere Attribut-Stücke in verschiedenen Records; jedes trägt die
    /// Gesamtgröße, aber nur das erste (Start-VCN 0) zählt.
    fn parse_data_attribute(attr: &[u8]) -> Option<u64> {
        // Non-resident flag
        let non_resident = attr.get(8)? != &0;

        if non_resident {
            // Start-VCN bei Offset 16; Real size bei Offset 48
            let start_vcn = u64::from_le_bytes(attr.get(16..24)?.try_into().ok()?);
            if start_vcn != 0 {
                return None;
            }
            Self::nonresident_real_size(attr)
        } else {
            // Bei resident: Content length bei Offset 16
            Some(u32::from_le_bytes([
                *attr.get(16)?,
                *attr.get(17)?,
                *attr.get(18)?,
                *attr.get(19)?,
            ]) as u64)
        }
    }

    /// Tatsächliche Größe eines non-resident Attributs (Offset 48)
    pub fn nonresident_real_size(attr: &[u8]) -> Option<u64> {
        if attr.get(8)? == &0 {
            return None;
        }
        Some(u64::from_le_bytes(attr.get(48..56)?.try_into().ok()?))
    }

    /// Dekodiert die Data Runs eines non-resident Attributs
    ///
    /// Jeder Run beginnt mit einem Header-Byte: die unteren 4 Bit sind die
    /// Byte-Länge des Längenfelds, die oberen 4 Bit die des Offsetfelds.
    /// Dann folgen die Länge in Clustern und der Cluster-Offset relativ zum
    /// vorherigen Run (vorzeichenbehaftet, Fragmente können rückwärts
    /// liegen). Header 0 beendet die Liste, ein Run ohne Offsetfeld ist sparse.
    pub fn parse_data_runs(attr: &[u8]) -> Option<Vec<DataRun>> {
        if attr.get(8)? == &0 {
            return None; // resident: keine Data Runs
        }

        // Offset der Run-Liste (Offset 32-33)
        let mut pos = u16::from_le_bytes([*attr.get(32)?, *attr.get(33)?]) as usize;
        let mut runs = Vec::new();
        let mut lcn: i64 = 0;

        loop {
            let header = *attr.get(pos)?;
            if header == 0 {
                break;
            }
            let length_size = (header & 0x0F) as usize;
            let offset_size = (header >> 4) as usize;
            if length_size == 0 || length_size > 8 || offset_size > 8 {
                return None;
            }
            pos += 1;

            let length = read_uint_le(attr.get(pos..pos + length_size)?);
            pos += length_size;
            if length == 0 {
                return None;
            }

            if offset_size == 0 {
                runs.push(DataRun { lcn: None, length });
                continue;
            }

            let delta = read_int_le(attr.get(pos..pos + offset_size)?);
            pos += offset_size;
            lcn = lcn.checked_add(delta)?;
            if lcn < 0 {
                return None;
            }
            runs.push(DataRun {
                lcn: Some(lcn as u64),
                length,
            });
        }

        Some(runs)
    }
}

/// Rangfolge der Namensvarianten: langer Name vor POSIX vor 8.3-Name
fn name_rank(namespace: u8) -> u8 {
    match namespace {
        file_name_namespace::WIN32 | file_name_namespace::WIN32_AND_DOS => 3,
        file_name_namespace::POSIX => 2,
        file_name_namespace::DOS => 1,
        _ => 0,
    }
}

/// Little-Endian-Zahl variabler Länge (1-8 Bytes), vorzeichenlos
fn read_uint_le(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .rev()
        .fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// Little-Endian-Zahl variabler Länge (1-8 Bytes), vorzeichenbehaftet
fn read_int_le(bytes: &[u8]) -> i64 {
    let value = read_uint_le(bytes);
    let bits = bytes.len() * 8;
    if bits < 64 && value & (1u64 << (bits - 1)) != 0 {
        (value | (!0u64 << bits)) as i64
    } else {
        value as i64
    }
}

/// Iterator über die Attribute eines Records
struct AttributeIter<'a> {
    record: &'a [u8],
    offset: usize,
}

impl<'a> Iterator for AttributeIter<'a> {
    type Item = (u32, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let record = self.record;
        if self.offset + 8 > record.len() {
            return None;
        }

        let attr_type = u32::from_le_bytes(record[self.offset..self.offset + 4].try_into().ok()?);
        if attr_type == attribute_types::END_MARKER {
            return None;
        }

        // Attribut-Länge (Offset 4-7 relativ zum Attribut)
        let attr_length =
            u32::from_le_bytes(record[self.offset + 4..self.offset + 8].try_into().ok()?) as usize;
        if attr_length < 16 || self.offset + attr_length > record.len() {
            return None;
        }

        let attr = &record[self.offset..self.offset + attr_length];
        self.offset += attr_length;
        Some((attr_type, attr))
    }
}

/// Informationen aus $STANDARD_INFORMATION
struct StandardInfo {
    is_hidden: bool,
    is_system: bool,
}

/// Informationen aus $FILE_NAME
struct FileNameAttr {
    parent_reference: u64,
    name: String,
    namespace: u8,
}

/// Synthetische MFT-Records für Tests (auch vom Reader benutzt)
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    pub const USN: [u8; 2] = [0x34, 0x12];

    /// Baut einen 1024-Byte-Record: FILE-Header, Update Sequence Array bei
    /// Offset 48 (USN + zwei Sektoren), Attribute ab Offset 56, End Marker.
    /// Die Sektorenden tragen bereits die USN, wie auf der Platte.
    pub fn build_record(flags: u16, attrs: &[Vec<u8>]) -> Vec<u8> {
        let mut record = vec![0u8; 1024];
        record[0..4].copy_from_slice(b"FILE");
        record[4..6].copy_from_slice(&48u16.to_le_bytes());
        record[6..8].copy_from_slice(&3u16.to_le_bytes());
        record[20..22].copy_from_slice(&56u16.to_le_bytes());
        record[22..24].copy_from_slice(&flags.to_le_bytes());
        record[48..50].copy_from_slice(&USN);

        let mut pos = 56;
        for attr in attrs {
            record[pos..pos + attr.len()].copy_from_slice(attr);
            pos += attr.len();
        }
        record[pos..pos + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());

        // Originalbytes der Sektorenden ins Array, USN an die Sektorenden
        let first_sector_end = [record[510], record[511]];
        let second_sector_end = [record[1022], record[1023]];
        record[50..52].copy_from_slice(&first_sector_end);
        record[52..54].copy_from_slice(&second_sector_end);
        record[510..512].copy_from_slice(&USN);
        record[1022..1024].copy_from_slice(&USN);
        record
    }

    /// Residentes Attribut mit 24-Byte-Header und Inhalt, auf 8 Bytes gerundet
    pub fn resident_attr(attr_type: u32, name_length: u8, content: &[u8]) -> Vec<u8> {
        let total = (24 + content.len()).div_ceil(8) * 8;
        let mut attr = vec![0u8; total];
        attr[0..4].copy_from_slice(&attr_type.to_le_bytes());
        attr[4..8].copy_from_slice(&(total as u32).to_le_bytes());
        attr[9] = name_length;
        attr[16..20].copy_from_slice(&(content.len() as u32).to_le_bytes());
        attr[20..22].copy_from_slice(&24u16.to_le_bytes());
        attr[24..24 + content.len()].copy_from_slice(content);
        attr
    }

    /// Non-residentes Attribut mit 64-Byte-Header, Data Runs ab Offset 64
    pub fn nonresident_attr(attr_type: u32, real_size: u64, runs: &[u8]) -> Vec<u8> {
        let total = (64 + runs.len()).div_ceil(8) * 8;
        let mut attr = vec![0u8; total];
        attr[0..4].copy_from_slice(&attr_type.to_le_bytes());
        attr[4..8].copy_from_slice(&(total as u32).to_le_bytes());
        attr[8] = 1;
        attr[32..34].copy_from_slice(&64u16.to_le_bytes());
        attr[48..56].copy_from_slice(&real_size.to_le_bytes());
        attr[64..64 + runs.len()].copy_from_slice(runs);
        attr
    }

    /// Inhalt eines $FILE_NAME-Attributs
    pub fn file_name_content(parent: u64, name: &str, namespace: u8) -> Vec<u8> {
        let utf16: Vec<u16> = name.encode_utf16().collect();
        let mut content = vec![0u8; 66 + utf16.len() * 2];
        content[0..8].copy_from_slice(&parent.to_le_bytes());
        content[64] = utf16.len() as u8;
        content[65] = namespace;
        for (i, unit) in utf16.iter().enumerate() {
            content[66 + i * 2..68 + i * 2].copy_from_slice(&unit.to_le_bytes());
        }
        content
    }

    pub fn file_name_attr(parent: u64, name: &str, namespace: u8) -> Vec<u8> {
        resident_attr(
            attribute_types::FILE_NAME,
            0,
            &file_name_content(parent, name, namespace),
        )
    }

    /// Macht aus einem Record einen Erweiterungs-Record von `base`
    /// (mit Sequenznummer 3 im oberen Wort, die ignoriert werden muss)
    pub fn make_extension(record: &mut [u8], base: u64) {
        record[32..40].copy_from_slice(&(base | (3u64 << 48)).to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn fixups_restore_sector_ends() {
        let mut record = build_record(0x01, &[]);
        // Sektorenden mit "Nutzdaten" belegen, dann wie NTFS durch die USN ersetzen
        record[50..52].copy_from_slice(&[0xBB, 0xAA]);
        record[52..54].copy_from_slice(&[0xDD, 0xCC]);

        assert!(MftParser::apply_fixups(&mut record));
        assert_eq!(&record[510..512], &[0xBB, 0xAA]);
        assert_eq!(&record[1022..1024], &[0xDD, 0xCC]);
    }

    #[test]
    fn fixups_reject_torn_record() {
        let mut record = build_record(0x01, &[]);
        record[1022] ^= 0xFF; // zweiter Sektor nicht fertig geschrieben
        assert!(!MftParser::apply_fixups(&mut record));
        assert!(!MftParser::apply_fixups(&mut vec![0u8; 1024]));
    }

    #[test]
    fn prefers_long_name_over_dos_name() {
        let mut record = build_record(
            0x03,
            &[
                file_name_attr(5, "PROGRA~1", file_name_namespace::DOS),
                file_name_attr(5, "Program Files", file_name_namespace::WIN32),
            ],
        );
        assert!(MftParser::apply_fixups(&mut record));

        let entry = MftParser::parse_record(&record, 100).unwrap();
        assert_eq!(entry.name, "Program Files");
        assert_eq!(entry.parent_reference, 5);
        assert_eq!(entry.mft_reference, 100);
        assert!(entry.is_directory);
    }

    #[test]
    fn only_unnamed_data_stream_counts() {
        let mut record = build_record(
            0x01,
            &[
                file_name_attr(5, "test.txt", file_name_namespace::WIN32_AND_DOS),
                resident_attr(attribute_types::DATA, 0, &[0u8; 10]),
                resident_attr(attribute_types::DATA, 15, &[0u8; 500]),
            ],
        );
        assert!(MftParser::apply_fixups(&mut record));

        let entry = MftParser::parse_record(&record, 7).unwrap();
        assert_eq!(entry.size, 10);
        assert!(!entry.is_directory);
    }

    #[test]
    fn nonresident_data_reports_real_size() {
        let mut record = build_record(
            0x01,
            &[
                file_name_attr(5, "big.bin", file_name_namespace::WIN32),
                nonresident_attr(attribute_types::DATA, 123_456_789, &[0x11, 0x08, 0x20, 0x00]),
            ],
        );
        assert!(MftParser::apply_fixups(&mut record));
        assert_eq!(MftParser::parse_record(&record, 8).unwrap().size, 123_456_789);
    }

    #[test]
    fn skips_unused_extension_and_nameless_records() {
        let name = file_name_attr(5, "x", file_name_namespace::WIN32);

        let mut unused = build_record(0x00, &[name.clone()]);
        assert!(MftParser::apply_fixups(&mut unused));
        assert!(MftParser::parse_record(&unused, 1).is_none());

        let mut extension = build_record(0x01, &[name]);
        make_extension(&mut extension, 42);
        assert!(MftParser::apply_fixups(&mut extension));
        assert!(MftParser::parse_record(&extension, 2).is_none());

        let mut nameless = build_record(0x01, &[resident_attr(attribute_types::DATA, 0, &[0u8; 4])]);
        assert!(MftParser::apply_fixups(&mut nameless));
        assert!(MftParser::parse_record(&nameless, 3).is_none());
    }

    #[test]
    fn extension_record_yields_its_attributes() {
        let mut extension = build_record(
            0x01,
            &[
                file_name_attr(5, "OneDrive.kg", file_name_namespace::WIN32),
                nonresident_attr(attribute_types::DATA, 5_000, &[0x11, 0x01, 0x20, 0x00]),
            ],
        );
        make_extension(&mut extension, 42);
        assert!(MftParser::apply_fixups(&mut extension));

        let Some(ParsedRecord::Extension(ext)) = MftParser::parse(&extension, 99) else {
            panic!("Erweiterungs-Record erwartet");
        };
        assert_eq!(ext.base_reference, 42);
        assert_eq!(ext.data_size, Some(5_000));
        let name = ext.file_name.unwrap();
        assert_eq!((name.name.as_str(), name.parent_reference, name.rank), ("OneDrive.kg", 5, 3));
    }

    #[test]
    fn only_first_data_piece_carries_size() {
        // Zweites Stück einer fragmentierten Datei: Start-VCN 8, Größe zählt nicht
        let mut piece = nonresident_attr(attribute_types::DATA, 9_999, &[0x11, 0x01, 0x20, 0x00]);
        piece[16..24].copy_from_slice(&8u64.to_le_bytes());
        let mut record = build_record(
            0x01,
            &[
                resident_attr(attribute_types::ATTRIBUTE_LIST, 0, &[0u8; 32]),
                file_name_attr(5, "frag.bin", file_name_namespace::WIN32),
                piece,
            ],
        );
        assert!(MftParser::apply_fixups(&mut record));
        assert_eq!(MftParser::parse_record(&record, 8).unwrap().size, 0);
    }

    #[test]
    fn nameless_base_with_attribute_list_is_placeholder() {
        let mut record = build_record(
            0x01,
            &[resident_attr(attribute_types::ATTRIBUTE_LIST, 0, &[0u8; 32])],
        );
        assert!(MftParser::apply_fixups(&mut record));

        let Some(ParsedRecord::Entry { entry, name_rank }) = MftParser::parse(&record, 8) else {
            panic!("Platzhalter erwartet");
        };
        assert_eq!(name_rank, 0);
        assert!(entry.name.is_empty());
        assert!(MftParser::parse_record(&record, 8).is_none());
    }

    #[test]
    fn decodes_data_runs() {
        // Beispiel aus der NTFS-Dokumentation: zwei Fragmente
        let attr = nonresident_attr(
            attribute_types::DATA,
            0,
            &[0x31, 0x38, 0x73, 0x25, 0x34, 0x32, 0x14, 0x01, 0xE5, 0x11, 0x02, 0x00],
        );
        assert_eq!(
            MftParser::parse_data_runs(&attr).unwrap(),
            vec![
                DataRun { lcn: Some(0x342573), length: 0x38 },
                DataRun { lcn: Some(0x342573 + 0x0211E5), length: 0x0114 },
            ]
        );

        // Negativer Offset: das zweite Fragment liegt vor dem ersten
        let attr = nonresident_attr(attribute_types::DATA, 0, &[0x11, 0x10, 0x20, 0x11, 0x02, 0xF8, 0x00]);
        assert_eq!(
            MftParser::parse_data_runs(&attr).unwrap(),
            vec![
                DataRun { lcn: Some(32), length: 16 },
                DataRun { lcn: Some(24), length: 2 },
            ]
        );

        // Sparse Run ohne Offsetfeld
        let attr = nonresident_attr(attribute_types::DATA, 0, &[0x11, 0x04, 0x10, 0x01, 0x03, 0x00]);
        assert_eq!(
            MftParser::parse_data_runs(&attr).unwrap(),
            vec![
                DataRun { lcn: Some(16), length: 4 },
                DataRun { lcn: None, length: 3 },
            ]
        );

        // Resident: keine Data Runs
        assert!(MftParser::parse_data_runs(&resident_attr(attribute_types::DATA, 0, &[1, 2, 3])).is_none());
    }

    #[test]
    fn attribute_iterator_stops_at_end_marker_and_bad_lengths() {
        let mut record = build_record(
            0x01,
            &[
                resident_attr(attribute_types::STANDARD_INFORMATION, 0, &[0u8; 48]),
                file_name_attr(5, "a", file_name_namespace::WIN32),
            ],
        );
        assert!(MftParser::apply_fixups(&mut record));

        let types: Vec<u32> = MftParser::attributes(&record).map(|(t, _)| t).collect();
        assert_eq!(types, vec![attribute_types::STANDARD_INFORMATION, attribute_types::FILE_NAME]);

        // Kaputte Länge im ersten Attribut: Iteration endet sofort
        record[56 + 4..56 + 8].copy_from_slice(&5000u32.to_le_bytes());
        assert_eq!(MftParser::attributes(&record).count(), 0);
    }
}
