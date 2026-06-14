//! Execution of HOST_ACTION packets. The keyboard only sends an opaque
//! action id; everything an action *does* is defined by the local config
//! allowlist. The HID value byte is never interpreted as a path or command.

use std::sync::{atomic::Ordering, Arc, Mutex};

use rawhid_host_core::config::{ActionBinding, HostActionKind};
use tauri::{AppHandle, Manager};

use crate::commands::{spawn_ai_refresh_watcher, MonitorExtras};
use crate::state::MonitorStatus;

pub enum ActionOutcome {
    Continue,
    /// The monitor loop should stop (handled like `MonitorCommand::Stop`;
    /// never call `stop_monitoring_internal` from inside the loop thread).
    StopRequested,
}

pub fn execute(
    app: &AppHandle,
    binding: &ActionBinding,
    _value: u8,
    extras: &MonitorExtras,
    status: &Arc<Mutex<MonitorStatus>>,
) -> Result<ActionOutcome, String> {
    match binding.action {
        HostActionKind::ShowWindow => {
            if let Some(window) = app.get_webview_window("main") {
                // unminimize() first: show()/set_focus() do not restore a window
                // that is minimized to the taskbar.
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            Ok(ActionOutcome::Continue)
        }
        // Triggered via HID, so the monitor loop is already running.
        HostActionKind::StartMonitoring => Ok(ActionOutcome::Continue),
        HostActionKind::StopMonitoring => Ok(ActionOutcome::StopRequested),
        HostActionKind::RefreshAiUsage => {
            if extras
                .ai_usage_refreshing
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return Err("refresh_in_progress".to_string());
            }
            let baseline = {
                let runtime = extras.ai_usage_runtime.lock().unwrap();
                let Some(runtime) = runtime.as_ref() else {
                    extras.ai_usage_refreshing.store(false, Ordering::SeqCst);
                    return Err("source_disabled".to_string());
                };
                let generation = runtime.shared().generation();
                if runtime.refresh().is_err() {
                    extras.ai_usage_refreshing.store(false, Ordering::SeqCst);
                    return Err("refresh_failed".to_string());
                }
                generation
            };
            spawn_ai_refresh_watcher(
                app.clone(),
                Arc::clone(&extras.config),
                Arc::clone(&extras.ai_usage_runtime),
                Arc::clone(status),
                Arc::clone(&extras.ai_usage_refreshing),
                baseline,
            );
            Ok(ActionOutcome::Continue)
        }
        HostActionKind::Launch => {
            let path = binding
                .path
                .as_deref()
                .ok_or_else(|| "launch path not configured".to_string())?;
            crate::app_launch::focus_or_launch(path, binding.match_exe.as_deref())?;
            Ok(ActionOutcome::Continue)
        }
        HostActionKind::OpenFolder => {
            let path = binding
                .path
                .as_deref()
                .ok_or_else(|| "open_folder path not configured".to_string())?;
            crate::explorer::open_folder(path, binding.prefer_tab)?;
            Ok(ActionOutcome::Continue)
        }
    }
}
