fn main() {
    // Slint UI kompilieren
    slint_build::compile("ui/main.slint").unwrap();

    // Windows: Manifest einbinden für Admin-Elevation
    #[cfg(target_os = "windows")]
    {
        embed_resource::compile("rustree.rc", embed_resource::NONE);
    }
}
