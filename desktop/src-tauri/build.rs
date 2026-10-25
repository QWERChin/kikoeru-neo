fn main() {
    // Each command gets an `allow-<name>` permission, which the
    // capabilities hand out per page origin (book/src/desktop.md).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "desktop_state",
            "use_local",
            "use_server",
            "switch_mode",
            "pick_folder",
            "import_discover",
            "import_preview",
            "import_run",
            "media_state",
            "open_log_dir",
        ]),
    ))
    .expect("tauri build configuration");
}
