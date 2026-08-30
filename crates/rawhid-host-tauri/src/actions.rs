//! Execution of HOST_ACTION packets. The keyboard only sends an opaque
//! action id; everything an action *does* is defined by the local config
//! allowlist. The HID value byte is never interpreted as a path or command.

use std::sync::{atomic::Ordering, Arc, Mutex};

use rawhid_host_core::config::{ActionBinding, HostActionKind};
use tauri::{AppHandle, Manager};

use crate::commands::{spawn_ai_refresh_watcher, MonitorExtras};
use crate::state::{AiDisplayTarget, MonitorStatus};

pub enum ActionOutcome {
    Continue,
    AiSessionSelected {
        label: String,
    },
    /// Automatic monitoring should stop while the Host Link worker remains
    /// available for discovery and keymap Config RPC.
    StopRequested,
}

pub fn execute(
    app: &AppHandle,
    binding: &ActionBinding,
    value: u8,
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
        HostActionKind::CycleAiSession => {
            let selected = extras
                .ai_display_slots
                .lock()
                .unwrap()
                .cycle_slot(value)
                .ok_or_else(|| "no_active_ai_sessions".to_string())?;
            Ok(ActionOutcome::AiSessionSelected {
                label: selected.label(),
            })
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
        HostActionKind::FocusAiTerminal => {
            // `value` is the ScreenKey's physical index, i.e. the display
            // slot to resolve. Anything short of "exactly one session
            // assigned to this slot" is a silent no-op per the design
            // (docs/screenkey-terminal-focus-design.md 3.5): out-of-range
            // slot, unassigned slot, and (unreachable in practice, see the
            // design's F11-F13) an empty terminal_target_id. None of these
            // touch `ai_terminal_focusing`: they never start a focus
            // sequence, so there is nothing to guard against re-entry.
            let assigned = extras
                .ai_display_slots
                .lock()
                .unwrap()
                .slots()
                .get(usize::from(value))
                .and_then(|slot| slot.assigned.clone());
            let terminal_target_id = match assigned {
                Some(AiDisplayTarget::Codex { terminal_target_id })
                | Some(AiDisplayTarget::Claude { terminal_target_id }) => terminal_target_id,
                None => return Ok(ActionOutcome::Continue),
            };
            if terminal_target_id.is_empty() {
                return Ok(ActionOutcome::Continue);
            }
            // Only claim the in-progress flag once we know a focus sequence
            // will actually run, so every early return above needs no
            // matching `store(false)`; `FocusGuard` releases it when the
            // spawned thread finishes.
            if extras
                .ai_terminal_focusing
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
            {
                return Err("focus_in_progress".to_string());
            }
            // The search-and-focus sequence runs on its own thread: the
            // monitor loop that calls `execute` must not block on it (it can
            // take ~1.1s when SetForegroundWindow is denied, see F2/F8).
            crate::ai_terminal_focus::spawn_focus(
                terminal_target_id,
                Arc::clone(&extras.ai_terminal_focusing),
            );
            Ok(ActionOutcome::Continue)
        }
    }
}
