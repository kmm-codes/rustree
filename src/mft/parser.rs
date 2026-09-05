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
//! # Wichtige Attribute
//!
//! - `$STANDARD_INFORMATION` (0x10): Timestamps, Flags
//! - `$FILE_NAME` (0x30): Dateiname und Parent-Referenz
//! - `$DATA` (0x80): Dateiinhalt oder Größeninformation

use super::types::{FileEntry, MftError};

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

/// Der MFT-Parser verarbeitet rohe MFT-Records
pub struct MftParser;

impl MftParser {
    /// Parst einen rohen MFT-Record und extrahiert die Dateiinformationen
    ///
    /// # Parameter
    /// - `record`: Die rohen Bytes des MFT-Records (normalerweise 1024 Bytes)
    /// - `mft_reference`: Die MFT-Referenznummer dieses Records
    ///
    /// # Rückgabe
    /// - `Some(FileEntry)` wenn der Record gültig ist
    /// - `None` wenn der Record leer/gelöscht ist
    pub fn parse_record(record: &[u8], mft_reference: u64) -> Option<FileEntry> {
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

        // Offset zum ersten Attribut (Offset 20-21)
        let first_attr_offset = u16::from_le_bytes([record[20], record[21]]) as usize;

        // Attribute durchlaufen
        let mut name = String::new();
        let mut parent_reference: u64 = 0;
        let mut size: u64 = 0;
        let mut is_hidden = false;
        let mut is_system = false;

        let mut offset = first_attr_offset;

        while offset + 4 <= record.len() {
            let attr_type = u32::from_le_bytes([
                record[offset], record[offset + 1],
                record[offset + 2], record[offset + 3],
            ]);

            if attr_type == attribute_types::END_MARKER {
                break;
            }

            // Attribut-Länge (Offset 4-7 relativ zum Attribut)
            let attr_length = u32::from_le_bytes([
                record[offset + 4], record[offset + 5],
                record[offset + 6], record[offset + 7],
            ]) as usize;

            if attr_length == 0 || offset + attr_length > record.len() {
                break;
            }

            match attr_type {
                attribute_types::STANDARD_INFORMATION => {
                    // $STANDARD_INFORMATION enthält Flags
                    if let Some(std_info) = Self::parse_standard_information(&record[offset..offset + attr_length]) {
                        is_hidden = std_info.is_hidden;
                        is_system = std_info.is_system;
                    }
                }
                attribute_types::FILE_NAME => {
                    // $FILE_NAME enthält Name und Parent-Referenz
                    if let Some(file_name_attr) = Self::parse_file_name(&record[offset..offset + attr_length]) {
                        // Wir bevorzugen den "langen" Namen (Namespace 0 oder 3)
                        if name.is_empty() || file_name_attr.namespace == 3 {
                            name = file_name_attr.name;
                            parent_reference = file_name_attr.parent_reference;
                        }
                    }
                }
                attribute_types::DATA => {
                    // $DATA enthält die Dateigröße
                    if let Some(data_size) = Self::parse_data_attribute(&record[offset..offset + attr_length]) {
                        size = data_size;
                    }
                }
                _ => {}
            }

            offset += attr_length;
        }

        // Ignoriere Records ohne Namen (Systemdaten)
        if name.is_empty() {
            return None;
        }

        Some(FileEntry {
            mft_reference,
            parent_reference,
            name,
            size,
            is_directory,
            is_hidden,
            is_system,
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

        // Parent-Referenz (erste 6 Bytes, Little Endian)
        let parent_reference = u64::from_le_bytes([
            content[0], content[1], content[2], content[3],
            content[4], content[5], 0, 0,
        ]);

        // Namespace (Offset 65)
        let namespace = *content.get(65)?;

        // Namenlänge in Characters (Offset 64)
        let name_length = *content.get(64)? as usize;

        // Name startet bei Offset 66 (UTF-16LE)
        let name_bytes = content.get(66..66 + name_length * 2)?;
        let name_u16: Vec<u16> = name_bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();

        let name = String::from_utf16_lossy(&name_u16);

        Some(FileNameAttr {
            parent_reference,
            name,
            namespace,
        })
    }

    /// Parst das $DATA Attribut um die Dateigröße zu ermitteln
    fn parse_data_attribute(attr: &[u8]) -> Option<u64> {
        // Non-resident flag
        let non_resident = attr.get(8)? != &0;

        if non_resident {
            // Bei non-resident: Real size bei Offset 48
            if attr.len() >= 56 {
                return Some(u64::from_le_bytes([
                    *attr.get(48)?, *attr.get(49)?, *attr.get(50)?, *attr.get(51)?,
                    *attr.get(52)?, *attr.get(53)?, *attr.get(54)?, *attr.get(55)?,
                ]));
            }
        } else {
            // Bei resident: Content length bei Offset 16
            if attr.len() >= 20 {
                return Some(u32::from_le_bytes([
                    *attr.get(16)?, *attr.get(17)?, *attr.get(18)?, *attr.get(19)?,
                ]) as u64);
            }
        }

        None
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
