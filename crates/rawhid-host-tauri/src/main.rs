#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|a| a == "--hud-focus-probe") {
        rawhid_host_tauri_lib::run_hud_focus_probe();
        return;
    }
    rawhid_host_tauri_lib::run();
}
