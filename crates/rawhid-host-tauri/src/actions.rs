//! Execution of HOST_ACTION packets. The keyboard only sends an opaque
//! action id; everything an action *does* is defined by the local config
//! allowlist. The HID value byte is never interpreted as a path or command.

use std::{
    sync::{atomic::Ordering, Arc, Mutex},
    time::Instant,
};

use rawhid_host_core::codex_activity::CodexSessionSnapshot;
use rawhid_host_core::config::{ActionBinding, HostActionKind};
use tauri::{AppHandle, Manager};

use crate::state::{AiDisplayTarget, MonitorStatus};
use crate::{
    commands::{respond_to_codex_approval_internal, spawn_ai_refresh_watcher, MonitorExtras},
    hud_coordinator::HudSelectionDirection,
};

pub enum ActionOutcome {
    Continue,
    /// A valid physical HUD action had no safe live target. It is deliberately
    /// non-fatal: stale packets, Claude slots, and double presses must never
    /// turn into a synthetic approval.
    HudNoop {
        reason: &'static str,
    },
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
        HostActionKind::HudPrevious => {
            let pending = extras.codex_activity.pending_approvals();
            let moved = extras
                .hud
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|hud| hud.move_selection(&pending, HudSelectionDirection::Previous));
            Ok(moved
                .map(|_| ActionOutcome::Continue)
                .unwrap_or(ActionOutcome::HudNoop {
                    reason: "no_selectable_hud_approval",
                }))
        }
        HostActionKind::HudNext => {
            let pending = extras.codex_activity.pending_approvals();
            let moved = extras
                .hud
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|hud| hud.move_selection(&pending, HudSelectionDirection::Next));
            Ok(moved
                .map(|_| ActionOutcome::Continue)
                .unwrap_or(ActionOutcome::HudNoop {
                    reason: "no_selectable_hud_approval",
                }))
        }
        HostActionKind::HudConfirm | HostActionKind::HudReject => {
            let pending = extras.codex_activity.pending_approvals();
            let reject = matches!(binding.action, HostActionKind::HudReject);
            let dispatch = extras
                .hud
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|hud| hud.begin_response(&pending, reject, Instant::now()));
            let Some(dispatch) = dispatch else {
                return Ok(ActionOutcome::HudNoop {
                    reason: if reject {
                        "hud_response_in_flight_or_no_decline"
                    } else {
                        "hud_response_in_flight_guard_or_no_selection"
                    },
                });
            };
            // `respond_to_approval` may wait for the App Server response.
            // Never make the Host Link monitor loop wait for that round trip.
            dispatch_hud_response(pending, extras.codex_broker.clone(), dispatch);
            Ok(ActionOutcome::Continue)
        }
        HostActionKind::SelectHudTarget => {
            let assigned = extras
                .ai_display_slots
                .lock()
                .unwrap()
                .slots()
                .get(usize::from(value))
                .and_then(|slot| slot.assigned.clone());
            let target = codex_target_for_slot(assigned, extras.codex_activity.snapshots());
            let Some((connection_id, thread_id)) = target else {
                return Ok(ActionOutcome::HudNoop {
                    reason: "slot_has_no_codex_pending_target",
                });
            };
            let pending = extras.codex_activity.pending_approvals();
            let selected = {
                let hud = extras.hud.lock().unwrap();
                hud.as_ref().is_some_and(|hud| {
                    hud.select_codex_thread(&pending, &connection_id, &thread_id)
                })
            };
            if !selected {
                return Ok(ActionOutcome::HudNoop {
                    reason: "codex_slot_has_no_pending_approval",
                });
            }
            // Render immediately, rather than waiting for the next periodic
            // update; `update` preserves this explicit target thereafter.
            if let Some(hud) = extras.hud.lock().unwrap().as_ref() {
                hud.update(app, &pending);
            }
            Ok(ActionOutcome::Continue)
        }
    }
}

fn codex_target_for_slot(
    assigned: Option<AiDisplayTarget>,
    snapshots: Vec<CodexSessionSnapshot>,
) -> Option<(String, String)> {
    let AiDisplayTarget::Codex { terminal_target_id } = assigned? else {
        return None;
    };
    snapshots
        .into_iter()
        .find(|snapshot| {
            snapshot.state.session_active
                && snapshot.is_display_target
                && snapshot.state.activity_state
                    == rawhid_host_core::packet::AiActivityState::WaitingApproval
                && snapshot.terminal_target_id == terminal_target_id
        })
        .map(|snapshot| (snapshot.owner_connection_id, snapshot.thread_id))
}

fn dispatch_hud_response(
    pending: Arc<rawhid_host_core::pending_approval::PendingApprovalStore>,
    broker: rawhid_host_core::codex_broker::CodexBrokerManager,
    dispatch: crate::hud_coordinator::HudResponseDispatch,
) {
    // `Builder::spawn` returns an error rather than panicking on the usual
    // OS thread-creation failure. In that case `dispatch` drops here and its
    // reservation is released, leaving a still-pending request retryable.
    let _ = std::thread::Builder::new()
        .name("hud-approval-response".to_string())
        .spawn(move || {
            // A duplicate press or a CLI-first response is expected to lose the
            // Broker first-wins race. The monitor already recorded dispatch, and
            // there is no safe recovery action for a stale physical packet.
            let _ = respond_to_codex_approval_internal(
                &pending,
                &broker,
                dispatch.selection.key.token(),
                dispatch.selection.decision_index,
            );
        });
}

fn spawn_response_task(task: impl FnOnce() + Send + 'static) {
    std::thread::spawn(task);
}

#[cfg(test)]
mod tests {
    use std::{sync::mpsc, time::Duration};

    use rawhid_host_core::{
        codex_activity::{AiClientStateSnapshot, CodexSessionSnapshot},
        packet::{AiActivityState, AiClientType, AiClientVariant, AiWorkPhase},
    };

    use crate::state::AiDisplayTarget;

    fn codex_snapshot(
        connection_id: &str,
        thread_id: &str,
        terminal_target_id: &str,
    ) -> CodexSessionSnapshot {
        CodexSessionSnapshot {
            thread_id: thread_id.to_string(),
            owner_connection_id: connection_id.to_string(),
            terminal_target_id: terminal_target_id.to_string(),
            registration_order: 1,
            state: AiClientStateSnapshot {
                client_type: AiClientType::Codex,
                client_variant: AiClientVariant::Cli,
                session_active: true,
                activity_state: AiActivityState::WaitingApproval,
                work_phase: AiWorkPhase::Unspecified,
                revision: 1,
            },
            is_display_target: true,
        }
    }

    #[test]
    fn response_dispatch_returns_before_the_response_work_finishes() {
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let before = std::time::Instant::now();
        super::spawn_response_task(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        assert!(before.elapsed() < Duration::from_millis(50));
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        release_tx.send(()).unwrap();
    }

    #[test]
    fn slot_selection_uses_the_display_targets_exact_connection_and_thread() {
        let target = super::codex_target_for_slot(
            Some(AiDisplayTarget::Codex {
                terminal_target_id: "terminal-b".to_string(),
            }),
            vec![
                codex_snapshot("connection-a", "thread-a", "terminal-a"),
                codex_snapshot("connection-b", "thread-b", "terminal-b"),
            ],
        );
        assert_eq!(
            target,
            Some(("connection-b".to_string(), "thread-b".to_string()))
        );
        assert!(super::codex_target_for_slot(
            Some(AiDisplayTarget::Claude {
                terminal_target_id: "claude".to_string(),
            }),
            Vec::new(),
        )
        .is_none());
    }

    #[test]
    fn slot_selection_keeps_thread_identity_when_one_connection_owns_two_threads() {
        let target = super::codex_target_for_slot(
            Some(AiDisplayTarget::Codex {
                terminal_target_id: "terminal-a".to_string(),
            }),
            vec![
                codex_snapshot("connection-a", "thread-a", "terminal-a"),
                codex_snapshot("connection-a", "thread-b", "terminal-b"),
            ],
        );

        assert_eq!(
            target,
            Some(("connection-a".to_string(), "thread-a".to_string()))
        );
    }
}
