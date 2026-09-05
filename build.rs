fn main() {
    // Slint UI kompilieren
    slint_build::compile("ui/main.slint").unwrap();

    // Windows: Manifest und Versionsinfo einbetten
    #[cfg(target_os = "windows")]
    embed_windows_resources();
}

/// Bettet das Manifest (Admin-Elevation, DPI-Awareness) und eine Versionsinfo
/// in die EXE ein. Das Resource-Skript wird hier erzeugt, damit die Version
/// nur an einer Stelle steht (Cargo.toml); Explorer und scripts/update.ps1
/// lesen sie aus der fertigen Datei.
///
/// `embed_resource::compile` linkt nur in Binaries, nicht in Tests: eine
/// Test-EXE mit Admin-Manifest ließe sich ohne Elevation nicht starten.
#[cfg(target_os = "windows")]
fn embed_windows_resources() {
    use std::{env, fs, path::PathBuf};

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = manifest_dir.join("rustree.manifest");

    // Slint meldet bereits rerun-if-changed, also müssen wir alles nennen,
    // was diese Ressourcen beeinflusst - Cargo.toml wegen der Version.
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed=Cargo.toml");

    let version = env::var("CARGO_PKG_VERSION").unwrap();
    let major = env::var("CARGO_PKG_VERSION_MAJOR").unwrap();
    let minor = env::var("CARGO_PKG_VERSION_MINOR").unwrap();
    let patch = env::var("CARGO_PKG_VERSION_PATCH").unwrap();
    let manifest = manifest.display().to_string().replace('\\', "\\\\");

    let rc = format!(
        r#"// Generiert von build.rs - nicht von Hand bearbeiten.
#include <windows.h>

// Manifest einbinden (ID 1 = CREATEPROCESS_MANIFEST_RESOURCE_ID)
1 RT_MANIFEST "{manifest}"

// Optional: App-Icon (später hinzufügen)
// 1 ICON "assets/rustree.ico"

VS_VERSION_INFO VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "Kevin Meister"
      VALUE "FileDescription", "rustree - Fast NTFS Disk Space Analyzer"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "rustree"
      VALUE "LegalCopyright", "Kevin Meister"
      VALUE "OriginalFilename", "rustree.exe"
      VALUE "ProductName", "rustree"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );

    let rc_path = out_dir.join("rustree.rc");
    fs::write(&rc_path, rc).unwrap();
    embed_resource::compile(&rc_path, embed_resource::NONE);
}
