fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "rc_window",
            "rc_notify",
            "rc_set_badge",
            "rc_open_external",
            "rc_rpc_ready",
            "rc_rpc_detectables",
            "rc_keybinds_set",
        ])),
    )
    .expect("failed to run tauri-build");
}
