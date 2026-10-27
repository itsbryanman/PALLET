fn main() {
    // Only these commands exist, and the capability file grants them
    // explicitly. No filesystem, shell, opener, or HTTP plugin is exposed.
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
        "boot",
        "status",
        "list_floor",
        "deliver",
        "deliver_batch",
        "undo",
        "duplicate",
        "save_incident",
        "tx_status",
        "history",
        "acknowledge",
        "choose_warehouse",
        "create_warehouse",
        "open_sample",
    ])))
    .expect("tauri build");
}
