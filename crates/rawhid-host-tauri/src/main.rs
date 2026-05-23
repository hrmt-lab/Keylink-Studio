#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    rawhid_host_tauri_lib::run();
}
