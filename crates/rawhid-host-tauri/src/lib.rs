mod actions;
mod app_launch;
mod commands;
mod explorer;
mod foreground;
mod icon;
mod startup;
mod state;

use state::AppState;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

pub fn run() {
    let (config, config_path) = commands::load_initial_config();
    let start_on_launch = config.app.start_monitoring_on_launch;
    let app_state = AppState::new(config, config_path);

    tauri::Builder::default()
        // Single-instance must be the first plugin registered.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::get_config_path,
            commands::save_config,
            commands::reload_config,
            commands::show_config_file_location,
            commands::get_status,
            commands::get_log_entries,
            commands::probe_devices,
            commands::probe_studio_devices,
            commands::read_studio_keymap,
            commands::start_monitoring,
            commands::stop_monitoring,
            commands::refresh_ai_usage,
            commands::get_running_apps,
            commands::get_app_icons,
            commands::get_launch_at_login,
            commands::set_launch_at_login,
            commands::get_key_stats,
            commands::list_key_stats_devices,
            commands::debug_inject_uplink,
        ])
        .setup(move |app| {
            setup_window_icon(app)?;
            setup_tray(app)?;
            if start_on_launch {
                let handle = app.handle().clone();
                let state = app.state::<AppState>();
                let _ = commands::begin_monitoring(handle, state.inner());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("failed to start RawHID Host");
}

fn setup_window_icon(app: &mut tauri::App) -> tauri::Result<()> {
    if let (Some(window), Some(icon)) = (app.get_webview_window("main"), app.default_window_icon())
    {
        window.set_icon(icon.clone())?;
    }

    Ok(())
}

fn setup_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let start = MenuItemBuilder::with_id("start", "Start monitoring").build(app)?;
    let stop = MenuItemBuilder::with_id("stop", "Stop monitoring").build(app)?;
    let show = MenuItemBuilder::with_id("show", "Show window").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&start, &stop])
        .separator()
        .items(&[&show, &quit])
        .build()?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("RawHID Host")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "start" => {
                let state = app.state::<AppState>();
                let _ = commands::begin_monitoring(app.clone(), state.inner());
            }
            "stop" => {
                let state = app.state::<AppState>();
                let _ = commands::stop_monitoring_internal(state.inner());
            }
            "show" => {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.unminimize();
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("main") {
                    if win.is_visible().unwrap_or(false) {
                        let _ = win.hide();
                    } else {
                        let _ = win.unminimize();
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                }
            }
        })
        .build(app)?;

    Ok(())
}
