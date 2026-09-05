//! Bridges `PendingApprovalStore` (`rawhid-host-core`) to the HUD window
//! (`hud_window.rs`) for the display side of
//! `docs/ai-approval-hud-design.md`. Stage 2 adds an opaque request token
//! to the payload so a separate Tauri command can answer the exact request.
//!
//! `HudCoordinator::update` is called once per host-link tick from
//! `commands.rs`'s monitor loop, after `drain_codex_state_changes` /
//! `drain_claude_state_changes` have fed both clients' unresolved-approval
//! bodies into the shared `PendingApprovalStore`
//! (`extras.codex_activity.pending_approvals()` -- see that call site's own
//! comment on why both clients share one store). It shows the newest
//! unresolved approval request and hides the HUD once none remain, per
//! §10: "HUDの表示自体はセッションごとではなく1つ。対象を切り替えて中身を
//! 差し替える" / "複数セッションが同時に承認待ちのときは、最新の1件を表示
//! する".
//!
//! This module never answers a request: it only reads `PendingApprovalStore`
//! and pushes a sanitized display payload to the `hud` webview via
//! `emit_to`. The Host Link packet, Firmware, and `claude_observer.rs`'s 204
//! response are all untouched (out of scope for stage 1, §13/§15 of the
//! design doc).

use std::{sync::Mutex, time::Instant};

use rawhid_host_core::pending_approval::{
    ApprovalClient, ApprovalKey, PendingApprovalContent, PendingApprovalSnapshot,
    PendingApprovalStore,
};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, WebviewUrl};

use crate::hud_window::HudWindow;

/// Window label, shared between window creation, the emitted event's
/// target, and `capabilities/default.json`'s `windows` entry.
pub const HUD_WINDOW_LABEL: &str = "hud";

const HUD_EVENT: &str = "hud-approval-update";

/// Logical (DPI-independent) HUD size and monitor margin. Converted to
/// physical pixels via the primary monitor's `scale_factor()` in
/// `hud_geometry` before every `show_at` call -- see
/// `docs/hud-focus-gate-results.md` §7-2: the KO-1 probe's 420x260 was a
/// *physical*-pixel size, which shrinks visually on a scaled display.
/// Values chosen to comfortably fit the §7.2 mockup's content (heading,
/// primary command, cwd, reason, decision list) without requiring the
/// panel to scroll in the common case.
const HUD_LOGICAL_WIDTH: f64 = 400.0;
const HUD_LOGICAL_HEIGHT: f64 = 300.0;
const HUD_LOGICAL_MARGIN: f64 = 20.0;

/// How long the panel's `.hud-leave` animation runs (`ui/src/hud/hud.css`).
/// The window stays visible for this long after the payload is cleared so
/// the animation can finish; keep the two in step.
const HUD_EXIT_ANIMATION: std::time::Duration = std::time::Duration::from_millis(160);

/// Sanitized, display-only view of one pending approval request, sent to
/// the HUD webview. Field names intentionally mirror
/// `PendingApprovalBody` / the comparison table in
/// `docs/ai-approval-hud-design.md` §7.2 so the frontend needs no separate
/// mapping table.
#[derive(Debug, Clone, Serialize)]
pub struct HudApprovalPayload {
    /// Opaque correlation token returned with a response. It contains no
    /// approval body or Broker credential.
    pub request_key: String,
    /// `"codex"` or `"claude_code"`.
    pub client: &'static str,
    /// `true` when the body exceeded `MAX_PENDING_APPROVAL_BODY_BYTES` and
    /// was not retained (`PendingApprovalContent::Oversized`); every other
    /// field is `None` in that case.
    pub oversized: bool,
    pub kind: Option<String>,
    pub primary_text: Option<String>,
    pub full_command: Option<String>,
    pub reason: Option<String>,
    pub cwd: Option<String>,
    /// Codex's `availableDecisions`, carried through unchanged (never
    /// reconstructed -- see `pending_approval.rs`'s doc comment on this
    /// field). Absent for Claude Code (§7.2: stage 1 shows nothing for it).
    pub available_decisions: Option<Vec<Value>>,
    /// The Host-managed position in `available_decisions`. `None` means
    /// there is no safe selectable decision (for example, an absent or empty
    /// array). The value is an index only; the opaque decision itself remains
    /// in `PendingApprovalStore`.
    pub selected_decision_index: Option<usize>,
}

impl HudApprovalPayload {
    fn from_snapshot(
        key: &ApprovalKey,
        snapshot: &PendingApprovalSnapshot,
        selected_decision_index: Option<usize>,
    ) -> Self {
        let client = client_label(snapshot.client);
        match &snapshot.content {
            PendingApprovalContent::Body(body) => Self {
                request_key: key.token().to_string(),
                client,
                oversized: false,
                kind: body.kind.clone(),
                primary_text: body.primary_text.clone(),
                full_command: body.full_command.clone(),
                reason: body.reason.clone(),
                cwd: body.cwd.clone(),
                available_decisions: body.available_decisions.clone(),
                selected_decision_index,
            },
            PendingApprovalContent::Oversized => Self {
                request_key: key.token().to_string(),
                client,
                oversized: true,
                kind: None,
                primary_text: None,
                full_command: None,
                reason: None,
                cwd: None,
                available_decisions: None,
                selected_decision_index: None,
            },
        }
    }
}

fn client_label(client: ApprovalClient) -> &'static str {
    match client {
        ApprovalClient::Codex => "codex",
        ApprovalClient::ClaudeCode => "claude_code",
    }
}

/// Direction for moving the Host-side HUD selection. This is intentionally a
/// pure state operation: converting a physical key into this direction and
/// forwarding a confirmed answer are later stages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HudSelectionDirection {
    Previous,
    Next,
}

/// An exact pending request and a safe index into that request's current
/// `availableDecisions` array. Both values are derived from Host state; no
/// approval id or decision value is reconstructed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudApprovalSelection {
    pub key: ApprovalKey,
    pub decision_index: usize,
}

/// Host-side interaction state for the approval currently visible in the
/// HUD. `shown_at` uses [`Instant`] so the later physical-input stage can
/// reject input during its 400 ms accidental-activation guard without being
/// affected by wall-clock changes.
#[derive(Debug, Clone, Default)]
pub struct HudInteractionState {
    target: Option<ApprovalKey>,
    selected_decision_index: Option<usize>,
    shown_at: Option<Instant>,
}

impl HudInteractionState {
    /// Synchronizes this state with the latest pending request. Only a
    /// changed opaque key resets the selection and display-start timestamp;
    /// ordinary periodic updates for the same request preserve both.
    pub fn sync_latest(
        &mut self,
        latest: Option<&ApprovalKey>,
        decision_count: usize,
        now: Instant,
    ) {
        if self.target.as_ref() == latest {
            return;
        }

        self.target = latest.cloned();
        match latest {
            Some(_) => {
                self.selected_decision_index = (decision_count > 0).then_some(0);
                self.shown_at = Some(now);
            }
            None => {
                self.selected_decision_index = None;
                self.shown_at = None;
            }
        }
    }

    /// Returns the currently selected index only if it is valid for the
    /// supplied live decision count. This makes absent/empty arrays and an
    /// unexpected same-request content update safe without changing state.
    pub fn selected_decision_index(&self, decision_count: usize) -> Option<usize> {
        self.selected_decision_index
            .filter(|index| *index < decision_count)
    }

    /// Moves the selection cyclically within the supplied live decision
    /// count. A missing target or zero decisions has no selectable result.
    pub fn move_selection(
        &mut self,
        decision_count: usize,
        direction: HudSelectionDirection,
    ) -> Option<usize> {
        if self.target.is_none() || decision_count == 0 {
            return None;
        }

        let next = match self.selected_decision_index(decision_count) {
            Some(current) => match direction {
                HudSelectionDirection::Previous => (current + decision_count - 1) % decision_count,
                HudSelectionDirection::Next => (current + 1) % decision_count,
            },
            None => 0,
        };
        self.selected_decision_index = Some(next);
        Some(next)
    }

    /// Returns the opaque target and currently safe index for a later
    /// confirmation path. It intentionally does not send a response.
    pub fn selected_approval(&self, decision_count: usize) -> Option<HudApprovalSelection> {
        Some(HudApprovalSelection {
            key: self.target.clone()?,
            decision_index: self.selected_decision_index(decision_count)?,
        })
    }

    pub fn shown_at(&self) -> Option<Instant> {
        self.shown_at
    }
}

fn snapshot_decision_count(snapshot: &PendingApprovalSnapshot) -> usize {
    match &snapshot.content {
        PendingApprovalContent::Body(body) => body.available_decisions.as_ref().map_or(0, Vec::len),
        PendingApprovalContent::Oversized => 0,
    }
}

/// Owns the HUD `WebviewWindow` and tracks which pending-approval entry (if
/// any) it currently shows. Created once at startup and shared with the
/// host-link monitor thread via `MonitorExtras` (`commands.rs`), so it must
/// stay `Send + Sync` -- see `HudWindow`'s own doc comment for why that
/// holds despite wrapping a raw `HWND`.
pub struct HudCoordinator {
    window: HudWindow,
    /// The key currently displayed, so a changed "latest" entry can release
    /// its predecessor's `set_protected` flag (`PendingApprovalStore`'s
    /// eviction guard) and so unchanged ticks don't re-issue `SetWindowPos`.
    shown: Mutex<Option<ApprovalKey>>,
    /// Selection and timing state used by the next physical-input stage.
    /// Kept independently from `shown` so it can expose small pure APIs
    /// without owning a Host Link action or an approval response path.
    interaction: Mutex<HudInteractionState>,
}

impl HudCoordinator {
    /// Creates the HUD window hidden, mirroring `hud_probe.rs`'s use of
    /// `HudWindow::create` at startup so WebView2 initialization happens at
    /// an inert moment (see `hud_window.rs`'s module doc). Must be called
    /// from a Tauri `setup()` callback, which is the only place an
    /// `AppHandle` capable of building windows is available before the
    /// host-link worker (and thus the first `update` call) can start.
    pub fn create(app: &AppHandle) -> Result<Self, String> {
        let window = HudWindow::create(app, HUD_WINDOW_LABEL, WebviewUrl::App("hud.html".into()))?;
        Ok(Self {
            window,
            shown: Mutex::new(None),
            interaction: Mutex::new(HudInteractionState::default()),
        })
    }

    /// Called once per host-link tick. Shows the newest unresolved approval
    /// request, or hides the HUD once none remain.
    pub fn update(&self, app: &AppHandle, pending: &PendingApprovalStore) {
        let mut shown = self.shown.lock().unwrap();
        match pending.latest() {
            Some((key, snapshot)) => {
                let changed = shown.as_ref() != Some(&key);
                if changed {
                    if let Some(previous) = shown.as_ref() {
                        pending.set_protected(previous, false);
                    }
                    pending.set_protected(&key, true);
                    *shown = Some(key.clone());
                }
                let selected_decision_index = {
                    let mut interaction = self.interaction.lock().unwrap();
                    let decision_count = snapshot_decision_count(&snapshot);
                    interaction.sync_latest(Some(&key), decision_count, Instant::now());
                    interaction.selected_decision_index(decision_count)
                };
                let payload = Some(HudApprovalPayload::from_snapshot(
                    &key,
                    &snapshot,
                    selected_decision_index,
                ));
                let _ = app.emit_to(HUD_WINDOW_LABEL, HUD_EVENT, payload);
                if changed {
                    self.show();
                }
            }
            None => {
                if let Some(previous) = shown.take() {
                    pending.set_protected(&previous, false);
                    self.interaction
                        .lock()
                        .unwrap()
                        .sync_latest(None, 0, Instant::now());
                    let empty: Option<HudApprovalPayload> = None;
                    let _ = app.emit_to(HUD_WINDOW_LABEL, HUD_EVENT, empty);
                    self.hide_after_exit_animation();
                }
            }
        }
    }

    /// Pure Host-state API for a future physical-input binding. The caller
    /// supplies the live decision count from the current snapshot; this
    /// method neither creates a `HOST_ACTION` nor sends a Broker response.
    #[allow(dead_code)] // Wired to physical input in the next HUD stage.
    pub fn move_selection(
        &self,
        decision_count: usize,
        direction: HudSelectionDirection,
    ) -> Option<usize> {
        self.interaction
            .lock()
            .unwrap()
            .move_selection(decision_count, direction)
    }

    /// Returns the current opaque target and safe decision index for a future
    /// confirmation binding. The actual first-wins Broker call remains out of
    /// scope for this stage.
    #[allow(dead_code)] // Wired to physical input in the next HUD stage.
    pub fn selected_approval(&self, decision_count: usize) -> Option<HudApprovalSelection> {
        self.interaction
            .lock()
            .unwrap()
            .selected_approval(decision_count)
    }

    /// Returns when the current opaque target began displaying. A future
    /// physical-input binding can compare this monotonic timestamp with its
    /// activation guard without accessing the interaction state directly.
    #[allow(dead_code)] // Read by the next physical-input stage.
    pub fn shown_at(&self) -> Option<Instant> {
        self.interaction.lock().unwrap().shown_at()
    }

    fn show(&self) {
        let (x, y, w, h) = hud_geometry(&self.window);
        self.window.show_at(x, y, w, h);
    }

    /// Hides the window once the panel's exit animation has had time to
    /// play (`.hud-leave` in `ui/src/hud/hud.css`).
    ///
    /// The wait runs on its own thread rather than inline: this is called
    /// from the Host Link tick loop, and sleeping there would stall device
    /// polling for the animation's duration. `HudWindow::hide` is a bare
    /// `ShowWindow(SW_HIDE)` on a raw HWND value, so it is safe to call
    /// from another thread.
    fn hide_after_exit_animation(&self) {
        let hwnd_raw = self.window.hwnd_raw();
        std::thread::spawn(move || {
            std::thread::sleep(HUD_EXIT_ANIMATION);
            HudWindow::hide_raw(hwnd_raw);
        });
    }
}

/// Computes the HUD's physical-pixel position/size for the primary
/// monitor's bottom-right corner, scaling the logical size/margin by the
/// monitor's `scale_factor()` (see this module's const doc comment). Falls
/// back to a fixed on-screen position if monitor info is unavailable, the
/// same fallback `hud_probe.rs`'s `bottom_right_position` uses.
fn hud_geometry(hud: &HudWindow) -> (i32, i32, i32, i32) {
    let fallback = (
        100,
        100,
        HUD_LOGICAL_WIDTH as i32,
        HUD_LOGICAL_HEIGHT as i32,
    );
    let Ok(Some(monitor)) = hud.window().primary_monitor() else {
        return fallback;
    };
    let scale = monitor.scale_factor();
    let w = (HUD_LOGICAL_WIDTH * scale).round() as i32;
    let h = (HUD_LOGICAL_HEIGHT * scale).round() as i32;
    let margin = (HUD_LOGICAL_MARGIN * scale).round() as i32;
    // Anchor to the work area, not the full monitor: `monitor.size()` spans
    // the whole screen including the taskbar, so a bottom-right HUD sits
    // underneath it. `rcWork` excludes the taskbar (and any other appbar)
    // wherever the user has docked it.
    let (area_x, area_y, area_w, area_h) = match work_area(hud) {
        Some(area) => area,
        None => {
            let pos = monitor.position();
            let size = monitor.size();
            (pos.x, pos.y, size.width as i32, size.height as i32)
        }
    };
    let x = area_x + area_w - w - margin;
    let y = area_y + area_h - h - margin;
    (x.max(area_x), y.max(area_y), w, h)
}

/// The monitor work area (screen minus taskbar/appbars) in physical
/// pixels, as `(x, y, width, height)`. Uses the monitor the HUD window
/// itself is on so a multi-monitor setup anchors to the right screen.
#[cfg(windows)]
fn work_area(hud: &HudWindow) -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        let monitor = MonitorFromWindow(hud.hwnd(), MONITOR_DEFAULTTOPRIMARY);
        GetMonitorInfoW(monitor, &mut info).as_bool()
    };
    if !ok {
        return None;
    }
    let work = info.rcWork;
    Some((
        work.left,
        work.top,
        work.right - work.left,
        work.bottom - work.top,
    ))
}

#[cfg(not(windows))]
fn work_area(_hud: &HudWindow) -> Option<(i32, i32, i32, i32)> {
    // No Win32 work-area concept here; the caller falls back to the full
    // monitor rectangle.
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ApprovalKey, HudInteractionState, HudSelectionDirection, Instant};

    #[test]
    fn new_target_starts_at_zero_and_records_display_time() {
        let key = ApprovalKey::new("approval-a");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();

        state.sync_latest(Some(&key), 2, shown_at);

        assert_eq!(state.selected_decision_index(2), Some(0));
        assert_eq!(state.shown_at(), Some(shown_at));
        assert_eq!(
            state.selected_approval(2).map(|selection| selection.key),
            Some(key)
        );
    }

    #[test]
    fn same_target_update_preserves_selection_and_display_time() {
        let key = ApprovalKey::new("approval-a");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_latest(Some(&key), 3, shown_at);
        assert_eq!(
            state.move_selection(3, HudSelectionDirection::Previous),
            Some(2)
        );

        state.sync_latest(Some(&key), 3, shown_at + Duration::from_secs(1));

        assert_eq!(state.selected_decision_index(3), Some(2));
        assert_eq!(state.shown_at(), Some(shown_at));
    }

    #[test]
    fn changed_target_resets_selection_and_display_time() {
        let first = ApprovalKey::new("approval-a");
        let second = ApprovalKey::new("approval-b");
        let first_shown_at = Instant::now();
        let second_shown_at = first_shown_at + Duration::from_millis(400);
        let mut state = HudInteractionState::default();
        state.sync_latest(Some(&first), 2, first_shown_at);
        assert_eq!(
            state.move_selection(2, HudSelectionDirection::Next),
            Some(1)
        );

        state.sync_latest(Some(&second), 2, second_shown_at);

        assert_eq!(state.selected_decision_index(2), Some(0));
        assert_eq!(state.shown_at(), Some(second_shown_at));
        assert_eq!(
            state.selected_approval(2).map(|selection| selection.key),
            Some(second)
        );
    }

    #[test]
    fn disappearing_target_clears_interaction_state() {
        let key = ApprovalKey::new("approval-a");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_latest(Some(&key), 1, shown_at);

        state.sync_latest(None, 0, shown_at + Duration::from_millis(1));

        assert_eq!(state.selected_decision_index(1), None);
        assert_eq!(state.selected_approval(1), None);
        assert_eq!(state.shown_at(), None);
    }

    #[test]
    fn empty_decisions_have_no_selectable_or_confirmable_value() {
        let key = ApprovalKey::new("approval-a");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();

        state.sync_latest(Some(&key), 0, shown_at);

        assert_eq!(state.selected_decision_index(0), None);
        assert_eq!(state.move_selection(0, HudSelectionDirection::Next), None);
        assert_eq!(state.selected_approval(0), None);
        assert_eq!(state.shown_at(), Some(shown_at));
    }
}
