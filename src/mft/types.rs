//! Datentypen für die MFT-Verarbeitung

use thiserror::Error;

/// Fehler die beim MFT-Zugriff auftreten können
#[derive(Error, Debug)]
pub enum MftError {
    #[error("Zugriff verweigert - Administrator-Rechte erforderlich")]
    AccessDenied,

    #[error("Laufwerk nicht gefunden: {0}")]
    DriveNotFound(String),

    #[error("Kein NTFS-Dateisystem: {0}")]
    NotNtfs(String),

    #[error("Fehler beim Lesen der MFT: {0}")]
    ReadError(String),

    #[error("Ungültiger MFT-Record")]
    InvalidRecord,

    #[error("Windows API Fehler: {0}")]
    WindowsError(#[from] windows::core::Error),

    #[error("IO Fehler: {0}")]
    IoError(#[from] std::io::Error),
}

/// Ein Eintrag aus der MFT - repräsentiert eine Datei oder einen Ordner
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Eindeutige MFT-Referenznummer
    pub mft_reference: u64,

    /// MFT-Referenz des Parent-Verzeichnisses
    pub parent_reference: u64,

    /// Dateiname (nur der Name, nicht der volle Pfad)
    pub name: String,

    /// Dateigröße in Bytes (0 für Verzeichnisse)
    pub size: u64,

    /// Ist dies ein Verzeichnis?
    pub is_directory: bool,

    /// Ist die Datei versteckt?
    pub is_hidden: bool,

    /// Ist dies eine Systemdatei?
    pub is_system: bool,
}

impl FileEntry {
    /// Erstellt einen neuen FileEntry
    pub fn new(
        mft_reference: u64,
        parent_reference: u64,
        name: String,
        size: u64,
        is_directory: bool,
    ) -> Self {
        Self {
            mft_reference,
            parent_reference,
            name,
            size,
            is_directory,
            is_hidden: false,
            is_system: false,
        }
    }

    /// Prüft ob dies der Root-Ordner ist (MFT-Referenz 5)
    pub fn is_root(&self) -> bool {
        // In NTFS hat das Root-Verzeichnis immer die MFT-Referenz 5
        self.mft_reference == 5
    }
}

/// Die wichtigsten MFT-System-Records
pub mod system_records {
    /// $MFT - Die MFT selbst
    pub const MFT: u64 = 0;
    /// $MFTMirr - Backup der ersten MFT-Records
    pub const MFT_MIRROR: u64 = 1;
    /// $LogFile - NTFS Journal
    pub const LOG_FILE: u64 = 2;
    /// $Volume - Volume-Informationen
    pub const VOLUME: u64 = 3;
    /// $AttrDef - Attribut-Definitionen
    pub const ATTR_DEF: u64 = 4;
    /// Root-Verzeichnis (das ist der Laufwerksbuchstabe, z.B. C:\)
    pub const ROOT: u64 = 5;
    /// $Bitmap - Cluster-Belegung
    pub const BITMAP: u64 = 6;
    /// $Boot - Boot-Sektor
    pub const BOOT: u64 = 7;
    /// $BadClus - Defekte Cluster
    pub const BAD_CLUS: u64 = 8;
}
