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

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use rawhid_host_core::pending_approval::{
    ApprovalClient, ApprovalKey, PendingApprovalBody, PendingApprovalContent,
    PendingApprovalSnapshot, PendingApprovalStore, PendingQuestion, CLAUDE_DECISION_ALLOW,
    CLAUDE_DECISION_ALLOW_WITH_PERMISSIONS, CLAUDE_DECISION_DENY,
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
/// Confirming immediately after the HUD appears is likely to be the tail of
/// the key press that opened or selected it. Reject remains deliberately
/// outside this guard as the user's escape route.
pub const HUD_CONFIRM_GUARD: std::time::Duration = std::time::Duration::from_millis(400);

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
    /// Display labels for `available_decisions`, same length and order,
    /// **Claude Code entries only** -- `None` for Codex, whose frontend
    /// fallback (`Hud.tsx`'s `decisionLabel`) is untouched by this field.
    /// This exists because Claude Code's `allow_with_permissions` element
    /// alone cannot tell the user what pressing it actually does: the same
    /// visible request can offer a suggestion whose `destination` is
    /// `"session"` (gone when the process exits) or `"localSettings"`
    /// (written to disk, permanent until edited by hand) with no other
    /// visual difference -- see `claude_allow_with_permissions_label`'s own
    /// doc comment for the exact wording rules. Labels are plain English
    /// regardless of the model-facing language, matching this HUD's
    /// existing English-only decision list.
    pub decision_labels: Option<Vec<String>>,
    pub interaction_kind: Option<String>,
    pub questions: Option<Vec<HudQuestion>>,
    pub question_index: Option<usize>,
    pub interaction_message: Option<String>,
    pub interaction_url: Option<String>,
    pub permission_text: Option<String>,
    pub requires_terminal: bool,
    pub physical_input_available: bool,
    pub review_mode: bool,
    pub review_answers: Option<Vec<HudAnswer>>,
    pub review_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HudQuestionOption {
    pub label: String,
    pub description: Option<String>,
    pub is_other: bool,
    pub is_secret: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HudQuestion {
    pub id: String,
    pub header: Option<String>,
    pub question: String,
    pub options: Vec<HudQuestionOption>,
    pub multi_select: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HudAnswer {
    pub id: String,
    pub value: String,
}

impl HudApprovalPayload {
    fn from_snapshot(
        key: &ApprovalKey,
        snapshot: &PendingApprovalSnapshot,
        selected_decision_index: Option<usize>,
        question_index: Option<usize>,
        review_mode: bool,
        review_answers: Option<Vec<HudAnswer>>,
        review_index: Option<usize>,
    ) -> Self {
        let client = client_label(snapshot.client);
        match &snapshot.content {
            PendingApprovalContent::Body(body) => {
                let terminal_only = body
                    .interaction
                    .as_ref()
                    .is_some_and(|interaction| interaction.requires_terminal);
                Self {
                    request_key: key.token().to_string(),
                    client,
                    oversized: false,
                    kind: body.kind.clone(),
                    primary_text: body.primary_text.clone(),
                    full_command: body.full_command.clone(),
                    reason: body.reason.clone(),
                    cwd: body.cwd.clone(),
                    available_decisions: (!terminal_only)
                        .then(|| body.available_decisions.clone())
                        .flatten(),
                    selected_decision_index,
                    decision_labels: (snapshot.client == ApprovalClient::ClaudeCode)
                        .then(|| claude_decision_labels(body))
                        .flatten(),
                    interaction_kind: body.interaction.as_ref().map(|value| {
                        serde_json::to_value(value.kind)
                            .ok()
                            .and_then(|value| value.as_str().map(str::to_string))
                            .unwrap_or_default()
                    }),
                    questions: body.interaction.as_ref().map(|interaction| {
                        interaction.questions.iter().map(hud_question).collect()
                    }),
                    question_index,
                    interaction_message: body
                        .interaction
                        .as_ref()
                        .and_then(|interaction| interaction.message.clone()),
                    interaction_url: body
                        .interaction
                        .as_ref()
                        .and_then(|interaction| interaction.url.clone()),
                    permission_text: body
                        .interaction
                        .as_ref()
                        .and_then(|interaction| interaction.permission_text.clone()),
                    requires_terminal: terminal_only,
                    physical_input_available: true,
                    review_mode,
                    review_answers,
                    review_index,
                }
            }
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
                decision_labels: None,
                interaction_kind: None,
                questions: None,
                question_index: None,
                interaction_message: None,
                interaction_url: None,
                permission_text: None,
                requires_terminal: true,
                physical_input_available: true,
                review_mode: false,
                review_answers: None,
                review_index: None,
            },
        }
    }
}

fn hud_question(question: &PendingQuestion) -> HudQuestion {
    HudQuestion {
        id: question.id.clone(),
        header: question.header.clone(),
        question: question.question.clone(),
        options: question
            .options
            .iter()
            .map(|option| HudQuestionOption {
                label: option.label.clone(),
                description: option.description.clone(),
                is_other: option.is_other,
                is_secret: option.is_secret,
            })
            .collect(),
        multi_select: question.multi_select,
    }
}

fn hud_answers(answers: &[(String, Value)]) -> Vec<HudAnswer> {
    answers
        .iter()
        .map(|(id, value)| HudAnswer {
            id: id.clone(),
            value: value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string()),
        })
        .collect()
}

/// Builds the exact per-index labels for a Claude Code entry's
/// `available_decisions` (see [`HudApprovalPayload::decision_labels`] for
/// why this is Claude Code only). `None` when there is nothing to label
/// (an absent or empty `available_decisions`), matching how the plain array
/// itself is `None` in that case.
fn claude_decision_labels(body: &PendingApprovalBody) -> Option<Vec<String>> {
    let decisions = body.available_decisions.as_ref()?;
    if decisions.is_empty() {
        return None;
    }
    Some(
        decisions
            .iter()
            .map(|decision| match decision.as_str() {
                Some(CLAUDE_DECISION_ALLOW) => "allow".to_string(),
                Some(CLAUDE_DECISION_DENY) => "deny".to_string(),
                Some(CLAUDE_DECISION_ALLOW_WITH_PERMISSIONS) => {
                    claude_allow_with_permissions_label(
                        body.permission_suggestions.as_deref().unwrap_or(&[]),
                    )
                }
                // Not expected for a Claude Code entry -- `claude_activity.rs`'s
                // `claude_approval_body` only ever emits the three known
                // strings above. Fall back to the raw JSON rather than
                // guessing at a label for something this Host never sent.
                _ => decision.to_string(),
            })
            .collect(),
    )
}

/// The label for the `allow_with_permissions` element, built purely from
/// `permission_suggestions` -- see [`HudApprovalPayload::decision_labels`]'s
/// doc comment for why the applied scope must always be visible here rather
/// than left for the user to guess.
///
/// Priority, per the design this implements (`docs/` is not modified by this
/// change, but the rule it captures is): a suggestion set is either entirely
/// `"session"`-scoped, contains at least one `"localSettings"` entry (the
/// disk-persisting case, called out first because it is the one that must
/// never look identical to the session-only case), or contains some other,
/// unrecognized destination value -- in which case that raw value is shown
/// verbatim rather than silently mapped to an invented phrase. Separately, a
/// `"setMode"` suggestion's `mode` is always appended when present, because
/// changing the session's own permission mode is a side effect distinct
/// from any single rule addition and must stay visible regardless of which
/// scope case above applies.
fn claude_allow_with_permissions_label(suggestions: &[Value]) -> String {
    let destinations: Vec<&str> = suggestions
        .iter()
        .filter_map(|suggestion| suggestion.get("destination").and_then(Value::as_str))
        .collect();
    let scope = if destinations.is_empty() {
        None
    } else if destinations
        .iter()
        .all(|destination| *destination == "session")
    {
        Some("this session".to_string())
    } else if destinations
        .iter()
        .any(|destination| *destination == "localSettings")
    {
        Some("saved to project settings".to_string())
    } else {
        // Unrecognized destination value(s) -- surface verbatim rather than
        // inventing a phrase for something this Host has never observed.
        Some(destinations.join(", "))
    };
    let mode = suggestions.iter().find_map(|suggestion| {
        if suggestion.get("type").and_then(Value::as_str) != Some("setMode") {
            return None;
        }
        suggestion.get("mode").and_then(Value::as_str)
    });
    match (scope, mode) {
        (Some(scope), Some(mode)) => format!("allow + always ({scope}, mode: {mode})"),
        (Some(scope), None) => format!("allow + always ({scope})"),
        (None, Some(mode)) => format!("allow + always (mode: {mode})"),
        (None, None) => "allow + always".to_string(),
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

/// The exact AI session the HUD currently targets, client-independent. This
/// is the single value `actions.rs`'s `hud_target_for_slot` and this
/// module's `target_session` deal in -- every "is this slot the HUD's
/// target" comparison in the Host must go through one of those two, not a
/// per-client pair of fields, so a fix to one client's targeting can never
/// again leave the other's display broken (see the 2026-09-04 handoff note).
///
/// Internal only, like `ApprovalKey::codex_thread`/`claude_session`: built
/// purely for Host-side slot/target matching, and its fields must never be
/// placed in a HUD payload or a Host Link packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HudTargetSession {
    Codex {
        connection_id: String,
        thread_id: String,
    },
    Claude {
        launch_id: String,
        session_id: String,
    },
}

/// An exact pending request and a safe index into that request's current
/// `availableDecisions` array. Both values are derived from Host state; no
/// approval id or decision value is reconstructed here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HudApprovalSelection {
    pub key: ApprovalKey,
    pub decision_index: usize,
    /// For a validated Codex input request, the JSON-RPC result to send.  It
    /// is absent for ordinary approval decisions and is built only from the
    /// retained question/options.
    pub response: Option<Value>,
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
    /// Exactly one physical HUD response may be in flight globally. It is
    /// intentionally not tied to `target`: the target can change while its
    /// previous Broker response is finishing, and that completion must never
    /// unlock a newer request.
    response_in_flight: Option<ApprovalKey>,
    question_index: usize,
    question_answers: Vec<(String, Value)>,
    question_option_index: usize,
    review_mode: bool,
    review_index: usize,
}

impl HudInteractionState {
    /// Synchronizes this state with the latest pending request. Only a
    /// changed opaque key resets the selection and display-start timestamp;
    /// ordinary periodic updates for the same request preserve both.
    pub fn sync_target(
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
                self.question_index = 0;
                self.question_answers.clear();
                self.question_option_index = 0;
                self.review_mode = false;
                self.review_index = 0;
            }
            None => {
                self.selected_decision_index = None;
                self.shown_at = None;
                self.question_index = 0;
                self.question_answers.clear();
                self.question_option_index = 0;
                self.review_mode = false;
                self.review_index = 0;
            }
        }
    }

    fn target(&self) -> Option<&ApprovalKey> {
        self.target.as_ref()
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

    /// Moves the selection to the reject side of the supplied live snapshot:
    /// an exact `"decline"` element if present, otherwise an exact
    /// `"cancel"` element, otherwise no move at all (see
    /// `reject_decision_index_from_body`'s doc comment for the priority
    /// order). A missing target is also a no-op. Like `move_selection`, this
    /// never constructs a response -- it only relocates the highlight.
    pub fn move_selection_toward_reject(
        &mut self,
        snapshot: &PendingApprovalSnapshot,
    ) -> Option<usize> {
        self.target.as_ref()?;
        let PendingApprovalContent::Body(body) = &snapshot.content else {
            return None;
        };
        let index = reject_decision_index_from_body(body)?;
        self.selected_decision_index = Some(index);
        Some(index)
    }

    /// Returns the opaque target and currently safe index for a later
    /// confirmation path. It intentionally does not send a response.
    pub fn selected_approval(&self, decision_count: usize) -> Option<HudApprovalSelection> {
        Some(HudApprovalSelection {
            key: self.target.clone()?,
            decision_index: self.selected_decision_index(decision_count)?,
            response: None,
        })
    }

    fn move_question_selection(
        &mut self,
        option_count: usize,
        direction: HudSelectionDirection,
    ) -> Option<usize> {
        if self.target.is_none() || option_count == 0 {
            return None;
        }
        self.question_option_index = match direction {
            HudSelectionDirection::Previous => {
                (self.question_option_index + option_count - 1) % option_count
            }
            HudSelectionDirection::Next => (self.question_option_index + 1) % option_count,
        };
        Some(self.question_option_index)
    }

    fn move_review_selection(
        &mut self,
        answer_count: usize,
        direction: HudSelectionDirection,
    ) -> Option<usize> {
        let item_count = answer_count.checked_add(1)?;
        if self.target.is_none() || item_count == 0 {
            return None;
        }
        self.review_index = match direction {
            HudSelectionDirection::Previous => (self.review_index + item_count - 1) % item_count,
            HudSelectionDirection::Next => (self.review_index + 1) % item_count,
        };
        Some(self.review_index)
    }

    pub fn shown_at(&self) -> Option<Instant> {
        self.shown_at
    }

    fn select_target(&mut self, target: &ApprovalKey, decision_count: usize, now: Instant) {
        self.sync_target(Some(target), decision_count, now);
    }
}

/// Calculates the selection fields sent to the HUD without mutating the
/// coordinator. Ordinary command/file approvals use the decision selection;
/// Codex Answer Deck input uses the active question's option selection;
/// terminal-only permissions/elicitation requests expose no selectable item.
fn hud_payload_selection_indices(
    snapshot: &PendingApprovalSnapshot,
    interaction: &HudInteractionState,
) -> (
    Option<usize>,
    Option<usize>,
    bool,
    Option<Vec<HudAnswer>>,
    Option<usize>,
) {
    let PendingApprovalContent::Body(body) = &snapshot.content else {
        return (None, None, false, None, None);
    };

    let Some(request) = body.interaction.as_ref() else {
        return (
            interaction.selected_decision_index(snapshot_decision_count(snapshot)),
            None,
            false,
            None,
            None,
        );
    };

    if request.requires_terminal {
        return (None, None, false, None, None);
    }

    match request.kind {
        rawhid_host_core::pending_approval::PendingRequestKind::Approval => (
            interaction.selected_decision_index(snapshot_decision_count(snapshot)),
            None,
            false,
            None,
            None,
        ),
        rawhid_host_core::pending_approval::PendingRequestKind::Input => {
            let selected = request
                .questions
                .get(interaction.question_index)
                .and_then(|question| {
                    (interaction.question_option_index < question.options.len())
                        .then_some(interaction.question_option_index)
                });
            let review = interaction.review_mode;
            (
                selected,
                Some(interaction.question_index),
                review,
                review.then(|| hud_answers(&interaction.question_answers)),
                review.then_some(interaction.review_index),
            )
        }
        rawhid_host_core::pending_approval::PendingRequestKind::Permissions
        | rawhid_host_core::pending_approval::PendingRequestKind::Elicitation => {
            (None, None, false, None, None)
        }
    }
}

/// Owns the one in-flight HUD response reservation. Dropping it releases the
/// reservation only when it still belongs to the same opaque request key.
/// This makes every worker return path -- including Broker errors and panic
/// unwinding -- eligible to re-enable a later physical response safely.
pub struct HudResponseDispatch {
    pub selection: HudApprovalSelection,
    _reservation: HudResponseReservation,
}

pub enum HudConfirmOutcome {
    Noop,
    Advanced,
    Dispatch(HudResponseDispatch),
}

struct HudResponseReservation {
    interaction: Arc<Mutex<HudInteractionState>>,
    key: ApprovalKey,
}

impl Drop for HudResponseReservation {
    fn drop(&mut self) {
        let mut interaction = self.interaction.lock().unwrap();
        if interaction.response_in_flight.as_ref() == Some(&self.key) {
            interaction.response_in_flight = None;
        }
    }
}

fn snapshot_decision_count(snapshot: &PendingApprovalSnapshot) -> usize {
    match &snapshot.content {
        PendingApprovalContent::Body(body) => {
            if body
                .interaction
                .as_ref()
                .is_some_and(|interaction| interaction.requires_terminal)
            {
                0
            } else {
                body.available_decisions.as_ref().map_or(0, Vec::len)
            }
        }
        PendingApprovalContent::Oversized => 0,
    }
}

fn current_target(
    pending: &PendingApprovalStore,
    explicit_target: Option<ApprovalKey>,
) -> Option<(ApprovalKey, PendingApprovalSnapshot)> {
    explicit_target
        .and_then(|key| pending.get(&key).map(|snapshot| (key, snapshot)))
        .or_else(|| pending.latest())
}

fn replace_shown(
    pending: &PendingApprovalStore,
    shown: &mut Option<ApprovalKey>,
    next: &ApprovalKey,
) -> bool {
    if shown.as_ref() == Some(next) {
        return false;
    }
    if let Some(previous) = shown.as_ref() {
        pending.set_protected(previous, false);
    }
    pending.set_protected(next, true);
    *shown = Some(next.clone());
    true
}

/// Confirm-only: returns the exact live selection to answer with, gated by
/// the 400 ms accidental-activation guard. Reject no longer sends a
/// response at all -- see `HudInteractionState::move_selection_toward_reject`
/// / `reject_decision_index_from_body` -- so this
/// function has nothing left to branch on.
pub(crate) fn response_selection_from_state(
    interaction: &HudInteractionState,
    pending: &PendingApprovalStore,
    now: Instant,
) -> Option<HudApprovalSelection> {
    if interaction.shown_at().is_none_or(|shown_at| {
        now.checked_duration_since(shown_at)
            .is_none_or(|elapsed| elapsed < HUD_CONFIRM_GUARD)
    }) {
        return None;
    }
    let key = interaction.target()?.clone();
    let snapshot = pending.get(&key)?;
    let decision_count = snapshot_decision_count(&snapshot);
    interaction.selected_approval(decision_count)
}

/// The reject side's decision index within the live `available_decisions`,
/// per the priority order in `docs/ai-approval-hud-design.md`: an exact
/// `"decline"` element wins, then an exact `"cancel"` element, then an exact
/// `"deny"` element, otherwise there is no reject-side target at all.
/// Elements are not necessarily strings, so this only ever matches via
/// `as_str()`.
///
/// `"deny"` is last and Codex-safe to add: stage 2's real-CLI capture
/// (`docs/codex-approval-proxy-gate-results.md` §5.1) never offered
/// `"deny"` alongside `"decline"`/`"cancel"`, so this addition cannot change
/// any existing Codex outcome -- it only gives Claude Code's Host-normalized
/// `[CLAUDE_DECISION_ALLOW, CLAUDE_DECISION_DENY]` (`pending_approval.rs`)
/// a reject-side target, since Claude Code has no `"cancel"`/`"decline"` of
/// its own.
fn reject_decision_index_from_body(body: &PendingApprovalBody) -> Option<usize> {
    let decisions = body.available_decisions.as_ref()?;
    decisions
        .iter()
        .position(|decision| decision.as_str() == Some("decline"))
        .or_else(|| {
            decisions
                .iter()
                .position(|decision| decision.as_str() == Some("cancel"))
        })
        .or_else(|| {
            decisions
                .iter()
                .position(|decision| decision.as_str() == Some("deny"))
        })
}

fn begin_response_from_state(
    interaction: Arc<Mutex<HudInteractionState>>,
    pending: &PendingApprovalStore,
    now: Instant,
) -> Option<HudResponseDispatch> {
    let mut state = interaction.lock().unwrap();
    if state.response_in_flight.is_some() {
        return None;
    }
    let selection = response_selection_from_state(&state, pending, now)?;
    state.response_in_flight = Some(selection.key.clone());
    drop(state);
    Some(HudResponseDispatch {
        _reservation: HudResponseReservation {
            interaction,
            key: selection.key.clone(),
        },
        selection,
    })
}

/// Shared core of `select_codex_thread`/`select_claude_session`: makes
/// `key`/`snapshot` (already resolved by the caller as the exact live
/// request to target) the explicit HUD selection. Factored out as a free
/// function, taking the interaction lock directly, so it -- and the
/// `target_session_from_state` counterpart below -- can be exercised in
/// tests without a `HudWindow`/`AppHandle`, the same reason
/// `begin_response_from_state` above is a free function rather than a
/// `HudCoordinator` method.
fn select_target_from_state(
    interaction: &Mutex<HudInteractionState>,
    key: &ApprovalKey,
    snapshot: &PendingApprovalSnapshot,
) {
    interaction.lock().unwrap().select_target(
        key,
        snapshot_decision_count(snapshot),
        Instant::now(),
    );
}

/// Shared core of `select_claude_session`, including the precondition lookup
/// itself (unlike `select_target_from_state`, which assumes the caller
/// already resolved a live `key`/`snapshot`) -- the same shape as
/// `begin_response_from_state` above, and for the same reason: a test
/// exercising "select an absent session" needs to observe the whole failure
/// path (the store lookup finding nothing, `false` returned, no state
/// mutated), not just the tail end of it. An absent entry leaves
/// `interaction` untouched: a failed selection must never clear or otherwise
/// disturb whatever target was already live.
fn select_claude_session_from_state(
    interaction: &Mutex<HudInteractionState>,
    pending: &PendingApprovalStore,
    launch_id: &str,
    session_id: &str,
) -> bool {
    let key = rawhid_host_core::pending_approval::claude_key(launch_id, session_id);
    let Some(snapshot) = pending.get(&key) else {
        return false;
    };
    select_target_from_state(interaction, &key, &snapshot);
    true
}

/// Shared core of `target_session`. See `select_target_from_state`'s doc
/// comment for why this is a free function taking the interaction lock
/// directly rather than a `HudCoordinator` method body.
fn target_session_from_state(interaction: &Mutex<HudInteractionState>) -> Option<HudTargetSession> {
    let interaction = interaction.lock().unwrap();
    let key = interaction.target()?;
    if let Some((connection_id, thread_id)) = key.codex_thread() {
        return Some(HudTargetSession::Codex {
            connection_id: connection_id.to_string(),
            thread_id: thread_id.to_string(),
        });
    }
    if let Some((launch_id, session_id)) = key.claude_session() {
        return Some(HudTargetSession::Claude {
            launch_id: launch_id.to_string(),
            session_id: session_id.to_string(),
        });
    }
    None
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
    interaction: Arc<Mutex<HudInteractionState>>,
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
            interaction: Arc::new(Mutex::new(HudInteractionState::default())),
        })
    }

    /// Called once per host-link tick. An explicitly selected live request
    /// remains displayed; otherwise the newest unresolved request is shown.
    /// A selected key that was resolved or evicted safely falls back to the
    /// current latest request (or hides when none remain).
    pub fn update(&self, app: &AppHandle, pending: &PendingApprovalStore) {
        self.update_with_device_availability(app, pending, true);
    }

    /// Updates the HUD and marks whether a verified Host Action device is
    /// currently available for physical input. A temporary zero-device state
    /// keeps the pending request visible while making the physical-control
    /// limitation explicit in the payload.
    pub fn update_with_device_availability(
        &self,
        app: &AppHandle,
        pending: &PendingApprovalStore,
        physical_input_available: bool,
    ) {
        let mut shown = self.shown.lock().unwrap();
        let selected = self.interaction.lock().unwrap().target().cloned();
        let current = current_target(pending, selected);
        match current {
            Some((key, snapshot)) => {
                let changed = replace_shown(pending, &mut shown, &key);
                let (
                    selected_decision_index,
                    question_index,
                    review_mode,
                    review_answers,
                    review_index,
                ) = {
                    let mut interaction = self.interaction.lock().unwrap();
                    let decision_count = snapshot_decision_count(&snapshot);
                    interaction.sync_target(Some(&key), decision_count, Instant::now());
                    hud_payload_selection_indices(&snapshot, &interaction)
                };
                let mut payload = HudApprovalPayload::from_snapshot(
                    &key,
                    &snapshot,
                    selected_decision_index,
                    question_index,
                    review_mode,
                    review_answers,
                    review_index,
                );
                payload.physical_input_available = physical_input_available;
                let payload = Some(payload);
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
                        .sync_target(None, 0, Instant::now());
                    let empty: Option<HudApprovalPayload> = None;
                    let _ = app.emit_to(HUD_WINDOW_LABEL, HUD_EVENT, empty);
                    self.hide_after_exit_animation();
                }
            }
        }
    }

    /// Moves the selected decision within the live selected request. This
    /// never answers the request.
    pub fn move_selection(
        &self,
        pending: &PendingApprovalStore,
        direction: HudSelectionDirection,
    ) -> Option<usize> {
        let mut interaction = self.interaction.lock().unwrap();
        let snapshot = pending.get(interaction.target()?)?;
        if let PendingApprovalContent::Body(body) = &snapshot.content {
            if let Some(request) = body.interaction.as_ref().filter(|request| {
                request.kind == rawhid_host_core::pending_approval::PendingRequestKind::Input
                    && !request.requires_terminal
            }) {
                if interaction.review_mode {
                    let answer_count = interaction.question_answers.len();
                    return interaction.move_review_selection(answer_count, direction);
                }
                let question = request.questions.get(interaction.question_index)?;
                return interaction.move_question_selection(question.options.len(), direction);
            }
        }
        interaction.move_selection(snapshot_decision_count(&snapshot), direction)
    }

    /// Handles the physical reject action for an input Answer Deck.  During
    /// question entry it goes to the previous question; on the first one it
    /// is a no-op.  During review it returns to the last question so the user
    /// can revise the deck before sending it.  Approval entries retain their
    /// existing reject-highlight behavior.
    pub fn reject(&self, pending: &PendingApprovalStore) -> Option<usize> {
        let mut interaction = self.interaction.lock().unwrap();
        let snapshot = pending.get(interaction.target()?)?;
        let PendingApprovalContent::Body(body) = &snapshot.content else {
            return None;
        };
        let Some(request) = body.interaction.as_ref() else {
            return interaction.move_selection_toward_reject(&snapshot);
        };
        if request.requires_terminal {
            // Permissions, elicitation, and unverified input shapes are
            // display-only. In particular, elicitation must not acquire a
            // HUD-side reject path merely because its body advertises a
            // string that happens to look like a decision.
            return None;
        }
        if request.kind != rawhid_host_core::pending_approval::PendingRequestKind::Input {
            return interaction.move_selection_toward_reject(&snapshot);
        }
        if interaction.review_mode {
            interaction.review_mode = false;
            interaction.question_index = request.questions.len().saturating_sub(1);
            interaction.question_option_index = request
                .questions
                .get(interaction.question_index)
                .and_then(|question| {
                    interaction
                        .question_answers
                        .iter()
                        .find(|(id, _)| id == &question.id)
                        .and_then(|(_, value)| value.as_str())
                        .and_then(|answer| {
                            question
                                .options
                                .iter()
                                .position(|option| option.label == answer)
                        })
                })
                .unwrap_or(0);
            return Some(interaction.question_index);
        }
        if interaction.question_index == 0 {
            return None;
        }
        interaction.question_index -= 1;
        interaction.question_option_index = request
            .questions
            .get(interaction.question_index)
            .and_then(|question| {
                interaction
                    .question_answers
                    .iter()
                    .find(|(id, _)| id == &question.id)
                    .and_then(|(_, value)| value.as_str())
                    .and_then(|answer| {
                        question
                            .options
                            .iter()
                            .position(|option| option.label == answer)
                    })
            })
            .unwrap_or(0);
        Some(interaction.question_index)
    }

    /// Atomically obtains the only physical-response reservation and returns
    /// its exact store-derived candidate. Confirm-only: Reject no longer
    /// sends anything (`move_selection_toward_reject` handles it), so a
    /// second Confirm press while one is already in flight becomes a benign
    /// no-op until the returned dispatch object is dropped.
    pub fn begin_response(
        &self,
        pending: &PendingApprovalStore,
        now: Instant,
    ) -> Option<HudResponseDispatch> {
        begin_response_from_state(Arc::clone(&self.interaction), pending, now)
    }

    /// Applies the Answer Deck confirm semantics.  Intermediate questions are
    /// stored locally and advance to the next question; only the final
    /// question creates a response reservation.
    pub fn confirm(&self, pending: &PendingApprovalStore, now: Instant) -> HudConfirmOutcome {
        let mut state = self.interaction.lock().unwrap();
        let Some(key) = state.target.clone() else {
            return HudConfirmOutcome::Noop;
        };
        if state.shown_at.is_none_or(|shown_at| {
            now.checked_duration_since(shown_at)
                .is_none_or(|elapsed| elapsed < HUD_CONFIRM_GUARD)
        }) || state.response_in_flight.is_some()
        {
            return HudConfirmOutcome::Noop;
        }
        let Some(snapshot) = pending.get(&key) else {
            return HudConfirmOutcome::Noop;
        };
        let PendingApprovalContent::Body(body) = snapshot.content else {
            return HudConfirmOutcome::Noop;
        };
        let Some(request) = body.interaction else {
            return HudConfirmOutcome::Noop;
        };
        if request.kind != rawhid_host_core::pending_approval::PendingRequestKind::Input
            || request.requires_terminal
        {
            return HudConfirmOutcome::Noop;
        }
        if state.review_mode {
            let send_index = state.question_answers.len();
            if state.review_index < send_index {
                let question_id = state.question_answers[state.review_index].0.clone();
                let Some(question_index) = request
                    .questions
                    .iter()
                    .position(|question| question.id == question_id)
                else {
                    return HudConfirmOutcome::Noop;
                };
                state.review_mode = false;
                state.question_index = question_index;
                state.question_option_index = state
                    .question_answers
                    .get(state.review_index)
                    .and_then(|(_, value)| value.as_str())
                    .and_then(|answer| {
                        request.questions[question_index]
                            .options
                            .iter()
                            .position(|option| option.label == answer)
                    })
                    .unwrap_or(0);
                return HudConfirmOutcome::Advanced;
            }
            if state.review_index != send_index {
                return HudConfirmOutcome::Noop;
            }
            let answers = state
                .question_answers
                .iter()
                .map(|(id, answer)| {
                    (
                        id.clone(),
                        serde_json::json!({ "answers": [answer.clone()] }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            let selection = HudApprovalSelection {
                key: key.clone(),
                decision_index: 0,
                response: Some(serde_json::json!({ "answers": answers })),
            };
            state.response_in_flight = Some(key.clone());
            drop(state);
            return HudConfirmOutcome::Dispatch(HudResponseDispatch {
                _reservation: HudResponseReservation {
                    interaction: Arc::clone(&self.interaction),
                    key,
                },
                selection,
            });
        }
        let Some(question) = request.questions.get(state.question_index) else {
            return HudConfirmOutcome::Noop;
        };
        let Some(option) = question.options.get(state.question_option_index) else {
            return HudConfirmOutcome::Noop;
        };
        if option.is_other || option.is_secret || question.multi_select {
            return HudConfirmOutcome::Noop;
        }
        state.question_answers.retain(|(id, _)| id != &question.id);
        state
            .question_answers
            .push((question.id.clone(), Value::String(option.label.clone())));
        if state.question_index + 1 < request.questions.len() {
            state.question_index += 1;
            state.question_option_index = 0;
            return HudConfirmOutcome::Advanced;
        }
        state.review_mode = true;
        state.review_index = 0;
        HudConfirmOutcome::Advanced
    }

    /// Makes the newest Codex approval belonging to this exact display
    /// connection and thread the explicit HUD target. Unassigned slots never
    /// reach this method (they fall back to `focus_ai_terminal_for_slot`
    /// before ever calling here -- see `actions.rs`'s `SelectHudTarget` arm),
    /// and an absent or threadless request is a benign no-op.
    pub fn select_codex_thread(
        &self,
        pending: &PendingApprovalStore,
        connection_id: &str,
        thread_id: &str,
    ) -> bool {
        let Some((key, snapshot)) =
            pending.latest_codex_for_connection_and_thread(connection_id, thread_id)
        else {
            return false;
        };
        select_target_from_state(&self.interaction, &key, &snapshot);
        true
    }

    /// Makes the one unresolved Claude Code request for this exact
    /// `(launch_id, session_id)` the explicit HUD target. Unlike
    /// `select_codex_thread`, this never needs to search
    /// `PendingApprovalStore` for "the newest" match: `claude_key` derives a
    /// single deterministic key from `(launch_id, session_id)` (one session
    /// holds at most one unresolved request -- see that function's own doc
    /// comment), so a direct `get` is exact by construction. An absent entry
    /// is a benign no-op, same as `select_codex_thread`.
    pub fn select_claude_session(
        &self,
        pending: &PendingApprovalStore,
        launch_id: &str,
        session_id: &str,
    ) -> bool {
        select_claude_session_from_state(&self.interaction, pending, launch_id, session_id)
    }

    pub fn target_requires_terminal(&self, pending: &PendingApprovalStore) -> bool {
        let interaction = self.interaction.lock().unwrap();
        let Some(key) = interaction.target() else {
            return false;
        };
        pending
            .get(key)
            .and_then(|snapshot| match snapshot.content {
                PendingApprovalContent::Body(body) => body.interaction,
                PendingApprovalContent::Oversized => None,
            })
            .is_some_and(|request| request.requires_terminal)
    }

    /// Returns when the current opaque target began displaying. A future
    /// physical-input binding can compare this monotonic timestamp with its
    /// activation guard without accessing the interaction state directly.
    #[allow(dead_code)] // Read by the next physical-input stage.
    pub fn shown_at(&self) -> Option<Instant> {
        self.interaction.lock().unwrap().shown_at()
    }

    /// The exact AI session currently shown as the HUD target, client-
    /// independent, for slot-vs-target comparisons (`actions.rs`'s
    /// `hud_target_for_slot`, `commands.rs`'s per-slot ScreenKey state
    /// calculation). `None` when nothing is shown, or the target's client
    /// identity could not be determined (a Codex request with an unknown
    /// thread id, or a key built via `ApprovalKey::new`, which should not
    /// happen in production for either client). Like `ApprovalKey::
    /// codex_thread`/`claude_session`, the returned value must never reach a
    /// HUD payload or a Host Link packet.
    pub fn target_session(&self) -> Option<HudTargetSession> {
        target_session_from_state(&self.interaction)
    }

    /// Returns the opaque key currently targeted by the HUD. This is used
    /// only for metadata-only audit records; callers must not expose the key's
    /// request body or answer state through this accessor.
    pub fn target_key(&self) -> Option<ApprovalKey> {
        self.interaction.lock().unwrap().target().cloned()
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
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use rawhid_host_core::pending_approval::{
        claude_key, codex_key, codex_key_for_thread, ApprovalClient, ApprovalOwner,
        PendingApprovalBody, PendingApprovalStore, PendingInteraction, PendingRequestKind,
    };
    use serde_json::{json, Value};

    use super::{
        begin_response_from_state, claude_allow_with_permissions_label, claude_decision_labels,
        current_target, hud_payload_selection_indices, reject_decision_index_from_body,
        replace_shown, response_selection_from_state, select_claude_session_from_state,
        select_target_from_state, target_session_from_state, ApprovalKey, HudInteractionState,
        HudSelectionDirection, HudTargetSession, Instant, HUD_CONFIRM_GUARD,
    };

    fn body(decisions: Vec<Value>) -> PendingApprovalBody {
        PendingApprovalBody {
            primary_text: None,
            full_command: None,
            reason: None,
            cwd: None,
            kind: None,
            available_decisions: Some(decisions),
            tool_use_id: None,
            prompt_id: None,
            permission_suggestions: None,
            interaction: None,
        }
    }

    fn insert_codex(
        store: &PendingApprovalStore,
        connection: &str,
        request: u64,
        decisions: Vec<Value>,
    ) -> ApprovalKey {
        let key = codex_key(connection, &json!(request));
        store.insert(
            key.clone(),
            ApprovalClient::Codex,
            ApprovalOwner::Codex {
                connection_id: connection.to_string(),
            },
            body(decisions),
        );
        key
    }

    #[test]
    fn new_target_starts_at_zero_and_records_display_time() {
        let key = ApprovalKey::new("approval-a");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();

        state.sync_target(Some(&key), 2, shown_at);

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
        state.sync_target(Some(&key), 3, shown_at);
        assert_eq!(
            state.move_selection(3, HudSelectionDirection::Previous),
            Some(2)
        );

        state.sync_target(Some(&key), 3, shown_at + Duration::from_secs(1));

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
        state.sync_target(Some(&first), 2, first_shown_at);
        assert_eq!(
            state.move_selection(2, HudSelectionDirection::Next),
            Some(1)
        );

        state.sync_target(Some(&second), 2, second_shown_at);

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
        state.sync_target(Some(&key), 1, shown_at);

        state.sync_target(None, 0, shown_at + Duration::from_millis(1));

        assert_eq!(state.selected_decision_index(1), None);
        assert_eq!(state.selected_approval(1), None);
        assert_eq!(state.shown_at(), None);
    }

    #[test]
    fn empty_decisions_have_no_selectable_or_confirmable_value() {
        let key = ApprovalKey::new("approval-a");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();

        state.sync_target(Some(&key), 0, shown_at);

        assert_eq!(state.selected_decision_index(0), None);
        assert_eq!(state.move_selection(0, HudSelectionDirection::Next), None);
        assert_eq!(state.selected_approval(0), None);
        assert_eq!(state.shown_at(), Some(shown_at));
    }

    #[test]
    fn physical_selection_wraps_without_constructing_a_response() {
        let key = ApprovalKey::new("approval-a");
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 3, Instant::now());

        assert_eq!(
            state.move_selection(3, HudSelectionDirection::Previous),
            Some(2)
        );
        assert_eq!(
            state.move_selection(3, HudSelectionDirection::Next),
            Some(0)
        );
        assert_eq!(state.selected_approval(3).unwrap().decision_index, 0);
    }

    #[test]
    fn ordinary_approval_payload_reflects_initial_next_previous_and_reject_selection() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            11,
            vec![json!("approve"), json!("cancel"), json!("decline")],
        );
        let snapshot = store.get(&key).expect("approval is pending");
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 3, shown_at);

        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(0));

        state.move_selection(3, HudSelectionDirection::Next);
        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(1));

        state.move_selection(3, HudSelectionDirection::Previous);
        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(0));

        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(2));
        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(2));
    }

    #[test]
    fn file_change_approval_payload_reflects_initial_next_previous_and_reject_selection() {
        let store = PendingApprovalStore::new();
        let key = codex_key("connection-a", &json!(12));
        let mut approval = body(vec![json!("approve"), json!("cancel"), json!("decline")]);
        approval.interaction = Some(PendingInteraction {
            kind: PendingRequestKind::Approval,
            questions: Vec::new(),
            message: None,
            url: None,
            permission_text: None,
            requires_terminal: false,
        });
        store.insert(
            key.clone(),
            ApprovalClient::Codex,
            ApprovalOwner::Codex {
                connection_id: "connection-a".to_string(),
            },
            approval,
        );

        let snapshot = store.get(&key).expect("approval is pending");
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 3, Instant::now());

        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(0));

        state.move_selection(3, HudSelectionDirection::Next);
        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(1));

        state.move_selection(3, HudSelectionDirection::Previous);
        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(0));

        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(2));
        let payload = hud_payload_for_test(&key, &snapshot, &state);
        assert_eq!(payload.selected_decision_index, Some(2));
    }

    fn hud_payload_for_test(
        key: &ApprovalKey,
        snapshot: &rawhid_host_core::pending_approval::PendingApprovalSnapshot,
        state: &HudInteractionState,
    ) -> super::HudApprovalPayload {
        let (selected, question_index, review, answers, review_index) =
            hud_payload_selection_indices(snapshot, state);
        super::HudApprovalPayload::from_snapshot(
            key,
            snapshot,
            selected,
            question_index,
            review,
            answers,
            review_index,
        )
    }

    #[test]
    fn answer_deck_review_selection_includes_trailing_send() {
        let key = ApprovalKey::new("input-a");
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 0, Instant::now());
        state.review_mode = true;
        state.question_answers = vec![
            ("first".to_string(), json!("safe")),
            ("second".to_string(), json!("fast")),
        ];

        assert_eq!(state.review_index, 0);
        assert_eq!(
            state.move_review_selection(2, HudSelectionDirection::Previous),
            Some(2),
            "previous from the first card selects the trailing Send item"
        );
        assert_eq!(
            state.move_review_selection(2, HudSelectionDirection::Next),
            Some(0)
        );
        assert_eq!(
            state.move_review_selection(2, HudSelectionDirection::Next),
            Some(1)
        );
    }

    #[test]
    fn confirm_guard_uses_the_live_selected_index() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            7,
            vec![json!("approve"), json!({"allow": true}), json!("decline")],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 3, shown_at);
        state.move_selection(3, HudSelectionDirection::Next);

        assert!(response_selection_from_state(
            &state,
            &store,
            shown_at + HUD_CONFIRM_GUARD - Duration::from_nanos(1)
        )
        .is_none());
        assert_eq!(
            response_selection_from_state(&state, &store, shown_at + HUD_CONFIRM_GUARD)
                .unwrap()
                .decision_index,
            1
        );
    }

    #[test]
    fn reject_with_only_cancel_moves_to_the_cancel_index() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            8,
            vec![json!("approve"), json!("cancel")],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 2, shown_at);

        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(1));
        assert_eq!(state.selected_decision_index(2), Some(1));
    }

    #[test]
    fn reject_prefers_decline_over_cancel() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            9,
            vec![json!("cancel"), json!("approve"), json!("decline")],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 3, shown_at);

        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(2));
    }

    #[test]
    fn reject_without_decline_or_cancel_does_not_move() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            10,
            vec![json!("approve"), json!({"allow": true})],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 2, shown_at);
        state.move_selection(2, HudSelectionDirection::Next);

        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), None);
        // The prior selection is left untouched by the no-op.
        assert_eq!(state.selected_decision_index(2), Some(1));
    }

    /// Claude Code's Host-normalized array has neither `"decline"` nor
    /// `"cancel"` -- `"deny"` is the only reject-side option it ever offers,
    /// which is exactly the case the third priority tier exists for.
    #[test]
    fn reject_falls_back_to_deny_for_claude_codes_allow_deny_array() {
        let store = PendingApprovalStore::new();
        let key = codex_key("connection-a", &json!(1)); // key shape is irrelevant to this pure function
        store.insert(
            key.clone(),
            ApprovalClient::ClaudeCode,
            ApprovalOwner::ClaudeSession {
                launch_id: "launch-1".to_string(),
                session_id: "session-1".to_string(),
            },
            body(vec![json!("allow"), json!("deny")]),
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 2, shown_at);

        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(1));
    }

    /// Regression: adding `"deny"` as a third priority tier must not change
    /// which index a Codex array with `"cancel"` already present resolves
    /// to -- `"cancel"` still wins over `"deny"` when both are present, even
    /// though real Codex captures never actually offer both together
    /// (`docs/codex-approval-proxy-gate-results.md` §5.1).
    #[test]
    fn reject_still_prefers_cancel_over_deny_when_a_codex_array_somehow_has_both() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            13,
            vec![json!("approve"), json!("cancel"), json!("deny")],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 3, shown_at);

        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(1));
    }

    #[test]
    fn reject_move_has_no_confirm_guard_delay() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            11,
            vec![json!("approve"), json!("decline")],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 2, shown_at);

        // `move_selection_toward_reject` takes no `now` argument at all: it
        // never consults `shown_at`/`HUD_CONFIRM_GUARD`, so the move succeeds
        // right at `shown_at` rather than only after 400 ms.
        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(1));
    }

    #[test]
    fn reject_move_then_confirm_sends_the_moved_to_index() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            12,
            vec![json!("approve"), json!("decline")],
        );
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        state.sync_target(Some(&key), 2, shown_at);

        let snapshot = store.get(&key).unwrap();
        assert_eq!(state.move_selection_toward_reject(&snapshot), Some(1));

        // Confirm still observes its own 400 ms guard for whatever is
        // selected, decline-moved-to or not.
        assert!(response_selection_from_state(
            &state,
            &store,
            shown_at + HUD_CONFIRM_GUARD - Duration::from_nanos(1)
        )
        .is_none());
        assert_eq!(
            response_selection_from_state(&state, &store, shown_at + HUD_CONFIRM_GUARD)
                .unwrap()
                .decision_index,
            1
        );
    }

    #[test]
    fn in_flight_response_allows_one_dispatch_then_releases_for_retry() {
        let store = PendingApprovalStore::new();
        let key = insert_codex(
            &store,
            "connection-a",
            7,
            vec![json!("approve"), json!("decline")],
        );
        let shown_at = Instant::now();
        let interaction = Arc::new(Mutex::new(HudInteractionState::default()));
        interaction
            .lock()
            .unwrap()
            .sync_target(Some(&key), 2, shown_at);

        let first = begin_response_from_state(
            Arc::clone(&interaction),
            &store,
            shown_at + HUD_CONFIRM_GUARD,
        )
        .expect("first dispatch reserves the HUD");
        assert!(begin_response_from_state(
            Arc::clone(&interaction),
            &store,
            shown_at + HUD_CONFIRM_GUARD,
        )
        .is_none());

        // Worker completion, Broker failure, and spawn failure all drop this
        // value; a still-pending request can then be retried.
        drop(first);
        assert!(begin_response_from_state(
            Arc::clone(&interaction),
            &store,
            shown_at + HUD_CONFIRM_GUARD,
        )
        .is_some());
    }

    #[test]
    fn old_response_reservation_cannot_unlock_a_newer_key() {
        let store = PendingApprovalStore::new();
        let first = insert_codex(&store, "connection-a", 1, vec![json!("approve")]);
        let second = insert_codex(&store, "connection-b", 2, vec![json!("approve")]);
        let shown_at = Instant::now();
        let interaction = Arc::new(Mutex::new(HudInteractionState::default()));
        interaction
            .lock()
            .unwrap()
            .sync_target(Some(&first), 1, shown_at);
        let first_dispatch = begin_response_from_state(
            Arc::clone(&interaction),
            &store,
            shown_at + HUD_CONFIRM_GUARD,
        )
        .unwrap();

        // This models a target transition racing with completion. The Drop
        // guard is key-scoped, so it never clears a reservation for `second`.
        interaction.lock().unwrap().response_in_flight = Some(second.clone());
        drop(first_dispatch);
        assert_eq!(interaction.lock().unwrap().response_in_flight, Some(second));
    }

    #[test]
    fn missing_empty_or_stale_targets_cannot_produce_a_response() {
        let store = PendingApprovalStore::new();
        let shown_at = Instant::now();
        let mut state = HudInteractionState::default();
        assert!(response_selection_from_state(&state, &store, shown_at).is_none());

        let empty = insert_codex(&store, "connection-a", 1, Vec::new());
        state.sync_target(Some(&empty), 0, shown_at);
        assert!(
            response_selection_from_state(&state, &store, shown_at + HUD_CONFIRM_GUARD).is_none()
        );
        store.resolve(&empty);
        assert!(
            response_selection_from_state(&state, &store, shown_at + HUD_CONFIRM_GUARD).is_none()
        );
    }

    #[test]
    fn explicit_target_is_retained_then_falls_back_and_releases_protection() {
        let store = PendingApprovalStore::new();
        let first = insert_codex(&store, "connection-a", 1, vec![json!("approve")]);
        let newest = insert_codex(&store, "connection-b", 2, vec![json!("approve")]);
        let mut shown = None;
        assert!(replace_shown(&store, &mut shown, &first));
        assert!(store.get(&first).unwrap().protected);

        // A periodic update keeps the explicitly selected request despite a
        // newer pending request.
        assert_eq!(
            current_target(&store, Some(first.clone())).unwrap().0,
            first
        );
        // Changing the displayed target transfers the eviction protection.
        assert!(replace_shown(&store, &mut shown, &newest));
        assert!(!store.get(&first).unwrap().protected);
        assert!(store.get(&newest).unwrap().protected);
        assert!(replace_shown(&store, &mut shown, &first));
        store.resolve(&first);
        // Once resolved, it immediately degrades to the latest live key.
        let fallback = current_target(&store, Some(first)).unwrap().0;
        assert_eq!(fallback, newest);
        assert!(replace_shown(&store, &mut shown, &fallback));
        assert!(store.get(&newest).unwrap().protected);
        // Resolving the target removes it despite protection; the next no-HUD
        // transition has no stale flag left to preserve.
        store.resolve(&newest);
        assert!(current_target(&store, shown.clone()).is_none());
    }

    /// `reject_decision_index_from_body`'s exact-match `"deny"` lookup must
    /// keep returning the last index once Claude Code's array grows a third,
    /// middle `allow_with_permissions` element -- this is what makes the
    /// design's "`deny` always last" placement rule actually matter, not
    /// just a cosmetic choice.
    #[test]
    fn reject_index_still_finds_deny_last_in_the_three_element_claude_array() {
        let three_element = body(vec![
            json!("allow"),
            json!("allow_with_permissions"),
            json!("deny"),
        ]);
        assert_eq!(reject_decision_index_from_body(&three_element), Some(2));
    }

    /// Pattern 1 of 4 from the design's required label test: every
    /// suggestion in the set is `"session"`-scoped.
    #[test]
    fn allow_with_permissions_label_for_all_session_scoped_suggestions() {
        let suggestions = vec![
            json!({"type": "addRules", "destination": "session", "rules": []}),
            json!({"type": "addRules", "destination": "session", "rules": []}),
        ];
        assert_eq!(
            claude_allow_with_permissions_label(&suggestions),
            "allow + always (this session)"
        );
    }

    /// Pattern 2 of 4: at least one `"localSettings"` entry, mixed with a
    /// `"session"` entry -- `localSettings` must win the label even though
    /// it is not every entry, since disk persistence is the safety-relevant
    /// fact the user needs to see.
    #[test]
    fn allow_with_permissions_label_prefers_local_settings_when_mixed_with_session() {
        let suggestions = vec![
            json!({"type": "addRules", "destination": "session", "rules": []}),
            json!({"type": "addRules", "destination": "localSettings", "rules": []}),
        ];
        assert_eq!(
            claude_allow_with_permissions_label(&suggestions),
            "allow + always (saved to project settings)"
        );
    }

    /// Pattern 3 of 4: an unrecognized `destination` value is surfaced
    /// verbatim, never silently mapped to "this session" or any other
    /// invented phrase.
    #[test]
    fn allow_with_permissions_label_surfaces_an_unknown_destination_verbatim() {
        let suggestions = vec![json!({
            "type": "addRules",
            "destination": "someFutureDestination",
            "rules": []
        })];
        assert_eq!(
            claude_allow_with_permissions_label(&suggestions),
            "allow + always (someFutureDestination)"
        );
    }

    /// Pattern 4 of 4: a `setMode` suggestion's `mode` is appended alongside
    /// the scope derived from the other entries in the same set -- the real
    /// two-entry `setMode` + `addDirectories` capture from the design.
    #[test]
    fn allow_with_permissions_label_appends_set_mode_alongside_the_scope() {
        let suggestions = vec![
            json!({"type": "setMode", "mode": "acceptEdits", "destination": "session"}),
            json!({"type": "addDirectories", "directories": ["C:\\temp"], "destination": "session"}),
        ];
        assert_eq!(
            claude_allow_with_permissions_label(&suggestions),
            "allow + always (this session, mode: acceptEdits)"
        );
    }

    /// End-to-end through `claude_decision_labels`: the three-element array
    /// gets `["allow", <computed allow_with_permissions label>, "deny"]`,
    /// same length and order as `available_decisions` itself.
    #[test]
    fn claude_decision_labels_covers_all_three_elements_in_order() {
        let suggestions = vec![json!({
            "type": "addRules",
            "destination": "localSettings",
            "rules": []
        })];
        let mut with_suggestions = body(vec![
            json!("allow"),
            json!("allow_with_permissions"),
            json!("deny"),
        ]);
        with_suggestions.permission_suggestions = Some(suggestions);

        assert_eq!(
            claude_decision_labels(&with_suggestions),
            Some(vec![
                "allow".to_string(),
                "allow + always (saved to project settings)".to_string(),
                "deny".to_string(),
            ])
        );
    }

    /// An absent or empty `available_decisions` has nothing to label.
    #[test]
    fn claude_decision_labels_is_none_without_available_decisions() {
        let mut no_decisions = body(vec![]);
        no_decisions.available_decisions = None;
        assert_eq!(claude_decision_labels(&no_decisions), None);

        let empty_decisions = body(vec![]);
        assert_eq!(claude_decision_labels(&empty_decisions), None);
    }

    fn insert_claude(
        store: &PendingApprovalStore,
        launch_id: &str,
        session_id: &str,
        decisions: Vec<Value>,
    ) -> ApprovalKey {
        let key = claude_key(launch_id, session_id);
        store.insert(
            key.clone(),
            ApprovalClient::ClaudeCode,
            ApprovalOwner::ClaudeSession {
                launch_id: launch_id.to_string(),
                session_id: session_id.to_string(),
            },
            body(decisions),
        );
        key
    }

    /// Client-independent targeting, Claude Code side, through
    /// `select_claude_session_from_state` itself (not just the pure state it
    /// wraps): selecting a real `(launch_id, session_id)` returns `true` and
    /// `target_session_from_state` then reports `HudTargetSession::Claude`;
    /// selecting a `(launch_id, session_id)` with no entry then returns
    /// `false` and leaves that same target in place. The second half is the
    /// property that actually matters -- a failed selection must never clear
    /// or replace whatever was already live -- so it must be checked as a
    /// continuation of the first selection, not against a fresh, empty
    /// `HudInteractionState` (which would pass even if `select_claude_session`
    /// mistakenly selected the wrong entry).
    #[test]
    fn select_claude_session_selects_a_real_session_and_leaves_it_alone_on_failure() {
        let store = PendingApprovalStore::new();
        insert_claude(&store, "launch-1", "session-1", vec![json!("allow")]);
        let interaction = Mutex::new(HudInteractionState::default());

        assert!(select_claude_session_from_state(
            &interaction,
            &store,
            "launch-1",
            "session-1",
        ));
        assert_eq!(
            target_session_from_state(&interaction),
            Some(HudTargetSession::Claude {
                launch_id: "launch-1".to_string(),
                session_id: "session-1".to_string(),
            })
        );

        // No entry exists for this pair, so `execute()`'s `SelectHudTarget`
        // arm would fall back to `focus_ai_terminal_for_slot` instead of
        // selecting -- and the previously selected session above must still
        // be the reported target.
        assert!(!select_claude_session_from_state(
            &interaction,
            &store,
            "launch-1",
            "session-missing",
        ));
        assert_eq!(
            target_session_from_state(&interaction),
            Some(HudTargetSession::Claude {
                launch_id: "launch-1".to_string(),
                session_id: "session-1".to_string(),
            })
        );
    }

    /// `target_session` reports the Codex variant for a Codex-selected
    /// target, matching `target_codex_thread`'s previous behavior before it
    /// was folded into this client-independent method. Uses
    /// `codex_key_for_thread` directly (rather than `insert_codex`'s plain
    /// `codex_key`) because only a key built with a known thread id has
    /// anything for `codex_thread`/`target_session` to report.
    #[test]
    fn target_session_reports_the_codex_variant_for_a_codex_target() {
        let store = PendingApprovalStore::new();
        let key = codex_key_for_thread("connection-a", &json!(1), Some("thread-a"));
        store.insert(
            key.clone(),
            ApprovalClient::Codex,
            ApprovalOwner::Codex {
                connection_id: "connection-a".to_string(),
            },
            body(vec![json!("approve")]),
        );
        let interaction = Mutex::new(HudInteractionState::default());

        let snapshot = store.get(&key).unwrap();
        select_target_from_state(&interaction, &key, &snapshot);

        assert_eq!(
            target_session_from_state(&interaction),
            Some(HudTargetSession::Codex {
                connection_id: "connection-a".to_string(),
                thread_id: "thread-a".to_string(),
            })
        );
    }
}
