use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    claude_hook_event::{ClaudeHookEvent, ClaudeObserverEvent},
    packet::{AiActivityState, AiWorkPhase},
};

pub const CLAUDE_DETAIL_STALE_TIMEOUT: Duration = Duration::from_secs(120);
const CLAUDE_COMPLETED_DISPLAY_DURATION: Duration = Duration::from_secs(30);
const CLAUDE_TOOL_TOMBSTONE_TTL: Duration = Duration::from_secs(120);
const MAX_TOOL_TOMBSTONES: usize = 256;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeStateChangeReason {
    SessionStarted,
    TurnStarted,
    ToolStarted,
    ToolCompleted,
    WaitingApproval,
    WaitingInput,
    InputResolved,
    TurnCompleted,
    CompletedExpired,
    TurnFailed,
    DetailStale,
    Desynchronized,
    SessionEnded,
    WrapperExited,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClaudeSessionSnapshot {
    pub launch_id: String,
    pub session_id: String,
    pub session_active: bool,
    pub activity_state: AiActivityState,
    pub work_phase: AiWorkPhase,
    pub desynchronized: bool,
    pub revision: u16,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClaudeStateChange {
    pub state: ClaudeSessionSnapshot,
    pub reason: ClaudeStateChangeReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaudeAdapterDiagnostic {
    MissingSessionId,
    MissingToolUseId,
    InvalidPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Approval,
    Input,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ClaudeCanonicalEvent {
    SessionStart,
    TurnStart,
    ToolStart {
        tool_use_id: String,
        phase: AiWorkPhase,
    },
    ToolComplete {
        tool_use_id: String,
    },
    ToolFailure {
        tool_use_id: String,
    },
    RequestStart {
        key: String,
        kind: RequestKind,
    },
    RequestResolve {
        key: String,
    },
    TurnComplete,
    TurnFailure,
    SessionEnd,
    WrapperExit,
}

/// Converts Claude Code hook payloads into events that do not retain answer text,
/// compact summaries, credentials, or other raw payload fields.
#[derive(Debug, Default)]
pub struct ClaudeEventAdapter;

impl ClaudeEventAdapter {
    fn adapt(
        &self,
        event: &ClaudeObserverEvent,
    ) -> Result<Option<ClaudeCanonicalEvent>, ClaudeAdapterDiagnostic> {
        match event {
            ClaudeObserverEvent::WrapperExited(_) => Ok(Some(ClaudeCanonicalEvent::WrapperExit)),
            ClaudeObserverEvent::Hook(hook) => self.adapt_hook(hook),
        }
    }

    fn adapt_hook(
        &self,
        hook: &ClaudeHookEvent,
    ) -> Result<Option<ClaudeCanonicalEvent>, ClaudeAdapterDiagnostic> {
        if hook.session_id.is_none() {
            return Err(ClaudeAdapterDiagnostic::MissingSessionId);
        }
        let tool_use_id = || required_string(&hook.body, "tool_use_id");
        match hook.hook_event_name.as_str() {
            "SessionStart" => Ok(Some(ClaudeCanonicalEvent::SessionStart)),
            "UserPromptSubmit" => Ok(Some(ClaudeCanonicalEvent::TurnStart)),
            "PreToolUse" => {
                let tool_use_id = tool_use_id().ok_or(ClaudeAdapterDiagnostic::MissingToolUseId)?;
                Ok(Some(ClaudeCanonicalEvent::ToolStart {
                    phase: tool_phase(required_string(&hook.body, "tool_name").as_deref()),
                    tool_use_id,
                }))
            }
            "PostToolUse" => Ok(Some(ClaudeCanonicalEvent::ToolComplete {
                tool_use_id: tool_use_id().ok_or(ClaudeAdapterDiagnostic::MissingToolUseId)?,
            })),
            "PostToolUseFailure" => Ok(Some(ClaudeCanonicalEvent::ToolFailure {
                tool_use_id: tool_use_id().ok_or(ClaudeAdapterDiagnostic::MissingToolUseId)?,
            })),
            "PermissionRequest" => Ok(Some(ClaudeCanonicalEvent::RequestStart {
                key: request_key(&hook.body, RequestKind::Approval),
                kind: RequestKind::Approval,
            })),
            "PermissionDenied" => Ok(Some(ClaudeCanonicalEvent::RequestResolve {
                key: request_key(&hook.body, RequestKind::Approval),
            })),
            "Elicitation" => Ok(Some(ClaudeCanonicalEvent::RequestStart {
                key: request_key(&hook.body, RequestKind::Input),
                kind: RequestKind::Input,
            })),
            "ElicitationResult" => Ok(Some(ClaudeCanonicalEvent::RequestResolve {
                key: request_key(&hook.body, RequestKind::Input),
            })),
            "Notification" => match notification_type(&hook.body) {
                Some("permission_prompt") => Ok(Some(ClaudeCanonicalEvent::RequestStart {
                    key: request_key(&hook.body, RequestKind::Approval),
                    kind: RequestKind::Approval,
                })),
                Some("elicitation_dialog") => Ok(Some(ClaudeCanonicalEvent::RequestStart {
                    key: request_key(&hook.body, RequestKind::Input),
                    kind: RequestKind::Input,
                })),
                _ => Ok(None),
            },
            "Stop" => Ok(Some(ClaudeCanonicalEvent::TurnComplete)),
            "StopFailure" => Ok(Some(ClaudeCanonicalEvent::TurnFailure)),
            "SessionEnd" => Ok(Some(ClaudeCanonicalEvent::SessionEnd)),
            _ => Ok(None),
        }
    }
}

pub struct ClaudeSessionReducer {
    adapter: ClaudeEventAdapter,
    snapshot: ClaudeSessionSnapshot,
    retired: bool,
    launch_ended: bool,
    turn_active: bool,
    requests: HashMap<String, RequestKind>,
    active_items: HashMap<String, AiWorkPhase>,
    tool_tombstones: VecDeque<(String, Instant)>,
    last_relevant_event: Option<Instant>,
    completed_deadline: Option<Instant>,
}

/// Owns all Claude Code sessions observed by Keylink Studio in stable
/// registration order. Cross-client display selection belongs to the host app.
pub struct ClaudeSessionRegistry {
    sessions: HashMap<(String, String), ClaudeSessionReducer>,
    order: Vec<(String, String)>,
    next_revision: u16,
}

impl Default for ClaudeSessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaudeSessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            order: Vec::new(),
            next_revision: 1,
        }
    }

    pub fn apply(
        &mut self,
        event: ClaudeObserverEvent,
        now: Instant,
    ) -> Result<Vec<ClaudeStateChange>, ClaudeAdapterDiagnostic> {
        match &event {
            ClaudeObserverEvent::WrapperExited(exit) => {
                let mut changes = Vec::new();
                for ((launch_id, _), reducer) in &mut self.sessions {
                    if launch_id == &exit.launch_id {
                        changes.extend(reducer.apply(event.clone(), now)?);
                    }
                }
                Ok(changes)
            }
            ClaudeObserverEvent::Hook(hook) => {
                let session_id = hook
                    .session_id
                    .as_deref()
                    .ok_or(ClaudeAdapterDiagnostic::MissingSessionId)?;
                let key = (hook.launch_id.clone(), session_id.to_string());
                if !self.sessions.contains_key(&key) {
                    if hook.hook_event_name != "SessionStart" {
                        return Ok(Vec::new());
                    }
                    let revision = self.allocate_revision();
                    self.sessions.insert(
                        key.clone(),
                        ClaudeSessionReducer::new(&hook.launch_id, session_id, revision),
                    );
                    self.order.push(key.clone());
                }
                let reducer = self.sessions.get_mut(&key).expect("inserted above");
                let changes = reducer.apply(event, now)?;
                Ok(changes)
            }
        }
    }

    pub fn tick(&mut self, now: Instant) -> Vec<ClaudeStateChange> {
        self.sessions
            .values_mut()
            .flat_map(|reducer| reducer.tick(now))
            .collect()
    }

    pub fn mark_launch_desynchronized(&mut self, launch_id: &str) -> Vec<ClaudeStateChange> {
        self.sessions
            .iter_mut()
            .filter(|((candidate, _), _)| candidate == launch_id)
            .flat_map(|(_, reducer)| reducer.mark_desynchronized())
            .collect()
    }

    pub fn snapshots(&self) -> Vec<ClaudeSessionSnapshot> {
        self.order
            .iter()
            .filter_map(|key| self.sessions.get(key).map(ClaudeSessionReducer::snapshot))
            .cloned()
            .collect()
    }

    fn allocate_revision(&mut self) -> u16 {
        let revision = self.next_revision;
        self.next_revision = self.next_revision.wrapping_add(1);
        revision
    }
}

impl ClaudeSessionReducer {
    pub fn new(launch_id: impl Into<String>, session_id: impl Into<String>, revision: u16) -> Self {
        Self {
            adapter: ClaudeEventAdapter,
            snapshot: ClaudeSessionSnapshot {
                launch_id: launch_id.into(),
                session_id: session_id.into(),
                session_active: false,
                activity_state: AiActivityState::None,
                work_phase: AiWorkPhase::Unspecified,
                desynchronized: false,
                revision,
            },
            retired: false,
            launch_ended: false,
            turn_active: false,
            requests: HashMap::new(),
            active_items: HashMap::new(),
            tool_tombstones: VecDeque::new(),
            last_relevant_event: None,
            completed_deadline: None,
        }
    }

    pub fn snapshot(&self) -> &ClaudeSessionSnapshot {
        &self.snapshot
    }

    pub fn apply(
        &mut self,
        event: ClaudeObserverEvent,
        now: Instant,
    ) -> Result<Vec<ClaudeStateChange>, ClaudeAdapterDiagnostic> {
        if !self.matches_event(&event) {
            return Ok(Vec::new());
        }
        self.prune_tombstones(now);
        let Some(event) = self.adapter.adapt(&event)? else {
            return Ok(Vec::new());
        };
        if self.launch_ended && event != ClaudeCanonicalEvent::WrapperExit {
            return Ok(Vec::new());
        }
        self.apply_canonical(event, now)
    }

    pub fn tick(&mut self, now: Instant) -> Vec<ClaudeStateChange> {
        self.prune_tombstones(now);
        if self
            .completed_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.completed_deadline = None;
            if self.snapshot.session_active
                && self.snapshot.activity_state == AiActivityState::Completed
            {
                return vec![self.emit(
                    AiActivityState::Available,
                    AiWorkPhase::Unspecified,
                    ClaudeStateChangeReason::CompletedExpired,
                )];
            }
        }
        if !self.snapshot.session_active || !self.turn_active {
            return Vec::new();
        }
        let Some(last_event) = self.last_relevant_event else {
            return Vec::new();
        };
        if now.duration_since(last_event) < CLAUDE_DETAIL_STALE_TIMEOUT
            || (self.snapshot.activity_state == AiActivityState::Working
                && self.snapshot.work_phase == AiWorkPhase::Unspecified)
        {
            return Vec::new();
        }
        vec![self.emit(
            AiActivityState::Working,
            AiWorkPhase::Unspecified,
            ClaudeStateChangeReason::DetailStale,
        )]
    }

    /// Records a receiver-side loss of ordering information (for example, a
    /// bounded queue overflow). This never invents a terminal state: an active
    /// turn remains `WORKING + UNSPECIFIED` until Claude supplies an end event.
    pub fn mark_desynchronized(&mut self) -> Vec<ClaudeStateChange> {
        if self.snapshot.desynchronized {
            return Vec::new();
        }
        self.snapshot.desynchronized = true;
        if !self.snapshot.session_active {
            return Vec::new();
        }
        let activity = if self.turn_active {
            AiActivityState::Working
        } else {
            self.snapshot.activity_state
        };
        vec![self.emit(
            activity,
            AiWorkPhase::Unspecified,
            ClaudeStateChangeReason::Desynchronized,
        )]
    }

    fn matches_event(&self, event: &ClaudeObserverEvent) -> bool {
        match event {
            ClaudeObserverEvent::WrapperExited(event) => event.launch_id == self.snapshot.launch_id,
            ClaudeObserverEvent::Hook(event) => {
                event.launch_id == self.snapshot.launch_id
                    && event.session_id.as_deref() == Some(self.snapshot.session_id.as_str())
            }
        }
    }

    fn apply_canonical(
        &mut self,
        event: ClaudeCanonicalEvent,
        now: Instant,
    ) -> Result<Vec<ClaudeStateChange>, ClaudeAdapterDiagnostic> {
        match event {
            ClaudeCanonicalEvent::SessionStart => {
                if self.launch_ended || self.snapshot.session_active {
                    return Ok(Vec::new());
                }
                self.retired = false;
                self.snapshot.desynchronized = false;
                self.last_relevant_event = Some(now);
                self.completed_deadline = None;
                Ok(vec![self.emit(
                    AiActivityState::Available,
                    AiWorkPhase::Unspecified,
                    ClaudeStateChangeReason::SessionStarted,
                )])
            }
            ClaudeCanonicalEvent::TurnStart => {
                if self.retired || !self.snapshot.session_active {
                    return Ok(Vec::new());
                }
                self.turn_active = true;
                self.requests.clear();
                self.active_items.clear();
                self.last_relevant_event = Some(now);
                self.completed_deadline = None;
                Ok(vec![self.emit(
                    AiActivityState::Working,
                    AiWorkPhase::Thinking,
                    ClaudeStateChangeReason::TurnStarted,
                )])
            }
            ClaudeCanonicalEvent::ToolStart { tool_use_id, phase } => {
                if self.retired || !self.snapshot.session_active || self.has_tombstone(&tool_use_id)
                {
                    return Ok(Vec::new());
                }
                self.turn_active = true;
                self.last_relevant_event = Some(now);
                if self.active_items.insert(tool_use_id, phase) == Some(phase)
                    && self.snapshot.activity_state == AiActivityState::Working
                    && self.snapshot.work_phase == phase
                {
                    return Ok(Vec::new());
                }
                Ok(vec![self.emit(
                    AiActivityState::Working,
                    self.active_phase(),
                    ClaudeStateChangeReason::ToolStarted,
                )])
            }
            ClaudeCanonicalEvent::ToolComplete { tool_use_id }
            | ClaudeCanonicalEvent::ToolFailure { tool_use_id } => {
                if self.retired || !self.snapshot.session_active {
                    return Ok(Vec::new());
                }
                self.last_relevant_event = Some(now);
                if self.has_tombstone(&tool_use_id) {
                    return Ok(Vec::new());
                }
                self.insert_tombstone(tool_use_id.clone(), now);
                let was_active = self.active_items.remove(&tool_use_id).is_some();
                self.requests.remove(&tool_use_id);
                if !was_active {
                    return Ok(Vec::new());
                }
                Ok(vec![self.emit(
                    self.waiting_or_working(),
                    self.active_phase(),
                    ClaudeStateChangeReason::ToolCompleted,
                )])
            }
            ClaudeCanonicalEvent::RequestStart { key, kind } => {
                if self.retired || !self.snapshot.session_active {
                    return Ok(Vec::new());
                }
                self.turn_active = true;
                self.last_relevant_event = Some(now);
                if self.requests.insert(key, kind).is_some() {
                    return Ok(Vec::new());
                }
                let reason = match kind {
                    RequestKind::Approval => ClaudeStateChangeReason::WaitingApproval,
                    RequestKind::Input => ClaudeStateChangeReason::WaitingInput,
                };
                Ok(vec![self.emit(
                    self.waiting_or_working(),
                    AiWorkPhase::Unspecified,
                    reason,
                )])
            }
            ClaudeCanonicalEvent::RequestResolve { key } => {
                if self.retired || !self.snapshot.session_active {
                    return Ok(Vec::new());
                }
                self.last_relevant_event = Some(now);
                if self.requests.remove(&key).is_none() {
                    return Ok(Vec::new());
                }
                Ok(vec![self.emit(
                    self.waiting_or_working(),
                    self.active_phase(),
                    ClaudeStateChangeReason::InputResolved,
                )])
            }
            ClaudeCanonicalEvent::TurnComplete | ClaudeCanonicalEvent::TurnFailure => {
                if self.retired || !self.snapshot.session_active || !self.turn_active {
                    return Ok(Vec::new());
                }
                let (activity, reason) = if event == ClaudeCanonicalEvent::TurnComplete {
                    self.completed_deadline = Some(now + CLAUDE_COMPLETED_DISPLAY_DURATION);
                    (
                        AiActivityState::Completed,
                        ClaudeStateChangeReason::TurnCompleted,
                    )
                } else {
                    self.completed_deadline = None;
                    (AiActivityState::Error, ClaudeStateChangeReason::TurnFailed)
                };
                self.finish_turn(now);
                Ok(vec![self.emit(activity, AiWorkPhase::Unspecified, reason)])
            }
            ClaudeCanonicalEvent::SessionEnd => {
                Ok(self.retire(ClaudeStateChangeReason::SessionEnded))
            }
            ClaudeCanonicalEvent::WrapperExit => {
                self.launch_ended = true;
                Ok(self.retire(ClaudeStateChangeReason::WrapperExited))
            }
        }
    }

    fn retire(&mut self, reason: ClaudeStateChangeReason) -> Vec<ClaudeStateChange> {
        if self.retired {
            return Vec::new();
        }
        self.retired = true;
        self.turn_active = false;
        self.requests.clear();
        self.active_items.clear();
        self.last_relevant_event = None;
        self.completed_deadline = None;
        if !self.snapshot.session_active {
            return Vec::new();
        }
        vec![self.emit(AiActivityState::None, AiWorkPhase::Unspecified, reason)]
    }

    fn finish_turn(&mut self, now: Instant) {
        let active_keys = self.active_items.keys().cloned().collect::<Vec<_>>();
        for key in active_keys {
            self.insert_tombstone(key, now);
        }
        self.turn_active = false;
        self.requests.clear();
        self.active_items.clear();
        self.last_relevant_event = None;
    }

    fn waiting_or_working(&self) -> AiActivityState {
        if self
            .requests
            .values()
            .any(|kind| *kind == RequestKind::Approval)
        {
            AiActivityState::WaitingApproval
        } else if self
            .requests
            .values()
            .any(|kind| *kind == RequestKind::Input)
        {
            AiActivityState::WaitingInput
        } else {
            AiActivityState::Working
        }
    }

    fn active_phase(&self) -> AiWorkPhase {
        self.active_items
            .values()
            .copied()
            .max_by_key(|phase| match phase {
                AiWorkPhase::Unspecified => 0,
                AiWorkPhase::Thinking => 1,
                AiWorkPhase::Executing => 2,
                AiWorkPhase::Searching => 3,
            })
            .unwrap_or(AiWorkPhase::Thinking)
    }

    fn has_tombstone(&self, tool_use_id: &str) -> bool {
        self.tool_tombstones
            .iter()
            .any(|(key, _)| key == tool_use_id)
    }

    fn insert_tombstone(&mut self, tool_use_id: String, now: Instant) {
        self.tool_tombstones.retain(|(key, _)| key != &tool_use_id);
        self.tool_tombstones.push_back((tool_use_id, now));
        while self.tool_tombstones.len() > MAX_TOOL_TOMBSTONES {
            self.tool_tombstones.pop_front();
        }
    }

    fn prune_tombstones(&mut self, now: Instant) {
        while self
            .tool_tombstones
            .front()
            .is_some_and(|(_, created)| now.duration_since(*created) >= CLAUDE_TOOL_TOMBSTONE_TTL)
        {
            self.tool_tombstones.pop_front();
        }
    }

    fn emit(
        &mut self,
        activity_state: AiActivityState,
        work_phase: AiWorkPhase,
        reason: ClaudeStateChangeReason,
    ) -> ClaudeStateChange {
        self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        self.snapshot.session_active = activity_state != AiActivityState::None;
        self.snapshot.activity_state = activity_state;
        self.snapshot.work_phase = if activity_state == AiActivityState::Working {
            work_phase
        } else {
            AiWorkPhase::Unspecified
        };
        ClaudeStateChange {
            state: self.snapshot.clone(),
            reason,
        }
    }
}

fn required_string(body: &Value, key: &str) -> Option<String> {
    body.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn notification_type(body: &Value) -> Option<&str> {
    body.get("notification_type")
        .or_else(|| body.get("notification"))
        .and_then(Value::as_str)
}

fn request_key(body: &Value, kind: RequestKind) -> String {
    match kind {
        RequestKind::Approval => required_string(body, "tool_use_id")
            .or_else(|| {
                required_string(body, "request_id").map(|value| format!("approval:{value}"))
            })
            .unwrap_or_else(|| "approval:unknown".to_string()),
        RequestKind::Input => required_string(body, "elicitation_id")
            .or_else(|| required_string(body, "request_id"))
            .map(|value| format!("input:{value}"))
            .unwrap_or_else(|| "input:unknown".to_string()),
    }
}

fn tool_phase(tool_name: Option<&str>) -> AiWorkPhase {
    match tool_name {
        Some("WebSearch") | Some("WebFetch") => AiWorkPhase::Searching,
        _ => AiWorkPhase::Executing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_hook_event::{ClaudeHookEvent, ClaudeObserverEvent, ClaudeWrapperExited};

    fn reducer() -> ClaudeSessionReducer {
        ClaudeSessionReducer::new("launch-1", "session-1", 100)
    }

    fn hook(name: &str, body: Value) -> ClaudeObserverEvent {
        hook_for_session("session-1", name, body)
    }

    fn hook_for_session(session_id: &str, name: &str, body: Value) -> ClaudeObserverEvent {
        hook_for_launch_session("launch-1", session_id, name, body)
    }

    fn hook_for_launch_session(
        launch_id: &str,
        session_id: &str,
        name: &str,
        body: Value,
    ) -> ClaudeObserverEvent {
        ClaudeObserverEvent::Hook(ClaudeHookEvent {
            launch_id: launch_id.to_string(),
            hook_event_name: name.to_string(),
            session_id: Some(session_id.to_string()),
            body,
        })
    }

    fn start_session(reducer: &mut ClaudeSessionReducer, now: Instant) {
        let change = reducer
            .apply(hook("SessionStart", serde_json::json!({})), now)
            .unwrap();
        assert_eq!(change[0].state.activity_state, AiActivityState::Available);
    }

    #[test]
    fn tool_completion_before_start_is_tombstoned() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        assert!(reducer
            .apply(
                hook("PostToolUse", serde_json::json!({"tool_use_id": "tool-a"})),
                now + Duration::from_secs(1),
            )
            .unwrap()
            .is_empty());
        assert!(reducer
            .apply(
                hook(
                    "PreToolUse",
                    serde_json::json!({"tool_use_id": "tool-a", "tool_name": "Bash"}),
                ),
                now + Duration::from_secs(2),
            )
            .unwrap()
            .is_empty());
        assert_eq!(reducer.snapshot().work_phase, AiWorkPhase::Thinking);
    }

    #[test]
    fn stale_only_downgrades_detail() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        reducer
            .apply(
                hook(
                    "PreToolUse",
                    serde_json::json!({"tool_use_id": "tool-a", "tool_name": "Bash"}),
                ),
                now,
            )
            .unwrap();
        assert!(reducer
            .tick(now + CLAUDE_DETAIL_STALE_TIMEOUT - Duration::from_millis(1))
            .is_empty());
        let changes = reducer.tick(now + CLAUDE_DETAIL_STALE_TIMEOUT);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].reason, ClaudeStateChangeReason::DetailStale);
        assert_eq!(changes[0].state.activity_state, AiActivityState::Working);
        assert_eq!(changes[0].state.work_phase, AiWorkPhase::Unspecified);
        assert!(changes[0].state.session_active);
    }

    #[test]
    fn completed_expires_after_display_duration() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        let completed = reducer
            .apply(hook("Stop", serde_json::json!({})), now)
            .unwrap();
        assert_eq!(
            completed[0].state.activity_state,
            AiActivityState::Completed
        );
        assert!(reducer
            .tick(now + CLAUDE_COMPLETED_DISPLAY_DURATION - Duration::from_millis(1))
            .is_empty());

        let expired = reducer.tick(now + CLAUDE_COMPLETED_DISPLAY_DURATION);
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].reason, ClaudeStateChangeReason::CompletedExpired);
        assert_eq!(expired[0].state.activity_state, AiActivityState::Available);
        assert_eq!(
            expired[0].state.revision,
            completed[0].state.revision.wrapping_add(1)
        );
        assert!(reducer
            .tick(now + CLAUDE_COMPLETED_DISPLAY_DURATION + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn starting_a_new_turn_cancels_completed_expiration() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        reducer
            .apply(hook("Stop", serde_json::json!({})), now)
            .unwrap();
        let working = reducer
            .apply(
                hook("UserPromptSubmit", serde_json::json!({})),
                now + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(working[0].state.activity_state, AiActivityState::Working);
        assert!(reducer
            .tick(now + CLAUDE_COMPLETED_DISPLAY_DURATION)
            .is_empty());
        assert_eq!(reducer.snapshot().activity_state, AiActivityState::Working);
    }

    #[test]
    fn manual_permission_never_guesses_execution() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        reducer
            .apply(
                hook(
                    "PreToolUse",
                    serde_json::json!({"tool_use_id": "tool-a", "tool_name": "Bash"}),
                ),
                now,
            )
            .unwrap();
        let changes = reducer
            .apply(
                hook(
                    "PermissionRequest",
                    serde_json::json!({"tool_use_id": "tool-a"}),
                ),
                now,
            )
            .unwrap();
        assert_eq!(
            changes[0].state.activity_state,
            AiActivityState::WaitingApproval
        );
        assert_eq!(changes[0].state.work_phase, AiWorkPhase::Unspecified);
        let changes = reducer
            .apply(
                hook("PostToolUse", serde_json::json!({"tool_use_id": "tool-a"})),
                now + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(changes[0].state.activity_state, AiActivityState::Working);
        assert_eq!(changes[0].state.work_phase, AiWorkPhase::Thinking);
        assert!(
            reducer
                .tick(now + Duration::from_secs(1) + CLAUDE_DETAIL_STALE_TIMEOUT)
                .len()
                == 1
        );
        assert_eq!(reducer.snapshot().activity_state, AiActivityState::Working);
        assert_eq!(reducer.snapshot().work_phase, AiWorkPhase::Unspecified);
    }

    #[test]
    fn session_end_and_wrapper_exit_are_idempotent() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        let ended = reducer
            .apply(hook("SessionEnd", serde_json::json!({})), now)
            .unwrap();
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].reason, ClaudeStateChangeReason::SessionEnded);
        let wrapper = ClaudeObserverEvent::WrapperExited(ClaudeWrapperExited {
            launch_id: "launch-1".to_string(),
            exit_code: 0,
        });
        assert!(reducer.apply(wrapper.clone(), now).unwrap().is_empty());
        assert!(reducer.apply(wrapper, now).unwrap().is_empty());
        assert!(reducer
            .apply(hook("SessionStart", serde_json::json!({})), now)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_session_can_resume_after_session_end_until_its_wrapper_exits() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("SessionEnd", serde_json::json!({})), now)
            .unwrap();
        let resumed = reducer
            .apply(
                hook("SessionStart", serde_json::json!({})),
                now + Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].reason, ClaudeStateChangeReason::SessionStarted);
        assert_eq!(resumed[0].state.activity_state, AiActivityState::Available);
    }

    #[test]
    fn desynchronization_keeps_an_active_turn_non_terminal_and_is_idempotent() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        let changes = reducer.mark_desynchronized();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].reason, ClaudeStateChangeReason::Desynchronized);
        assert_eq!(changes[0].state.activity_state, AiActivityState::Working);
        assert_eq!(changes[0].state.work_phase, AiWorkPhase::Unspecified);
        assert!(changes[0].state.desynchronized);
        assert!(reducer.mark_desynchronized().is_empty());
    }

    #[test]
    fn wrapper_exit_retires_active_session_when_session_end_is_missing() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        let changes = reducer
            .apply(
                ClaudeObserverEvent::WrapperExited(ClaudeWrapperExited {
                    launch_id: "launch-1".to_string(),
                    exit_code: 9,
                }),
                now,
            )
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].reason, ClaudeStateChangeReason::WrapperExited);
        assert!(!changes[0].state.session_active);
    }

    #[test]
    fn session_start_for_a_different_session_is_not_applied() {
        let now = Instant::now();
        let mut reducer = reducer();
        let event = ClaudeObserverEvent::Hook(ClaudeHookEvent {
            launch_id: "launch-1".to_string(),
            hook_event_name: "SessionStart".to_string(),
            session_id: Some("session-2".to_string()),
            body: serde_json::json!({}),
        });
        assert!(reducer.apply(event, now).unwrap().is_empty());
        assert!(!reducer.snapshot().session_active);
    }

    #[test]
    fn raw_payload_requires_session_and_tool_identity() {
        let adapter = ClaudeEventAdapter;
        let missing_session = ClaudeObserverEvent::Hook(ClaudeHookEvent {
            launch_id: "launch-1".to_string(),
            hook_event_name: "PreToolUse".to_string(),
            session_id: None,
            body: serde_json::json!({"tool_use_id": "tool-a"}),
        });
        assert_eq!(
            adapter.adapt(&missing_session),
            Err(ClaudeAdapterDiagnostic::MissingSessionId)
        );
        let missing_tool = hook("PreToolUse", serde_json::json!({"tool_name": "Bash"}));
        assert_eq!(
            adapter.adapt(&missing_tool),
            Err(ClaudeAdapterDiagnostic::MissingToolUseId)
        );
    }

    #[test]
    fn tombstone_expires_after_the_detail_stale_window() {
        let now = Instant::now();
        let mut reducer = reducer();
        start_session(&mut reducer, now);
        reducer
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        reducer
            .apply(
                hook("PostToolUse", serde_json::json!({"tool_use_id": "tool-a"})),
                now,
            )
            .unwrap();
        assert!(reducer
            .apply(
                hook(
                    "PreToolUse",
                    serde_json::json!({"tool_use_id": "tool-a", "tool_name": "Bash"}),
                ),
                now + Duration::from_secs(1),
            )
            .unwrap()
            .is_empty());
        let changes = reducer
            .apply(
                hook(
                    "PreToolUse",
                    serde_json::json!({"tool_use_id": "tool-a", "tool_name": "Bash"}),
                ),
                now + CLAUDE_TOOL_TOMBSTONE_TTL,
            )
            .unwrap();
        assert_eq!(changes[0].state.work_phase, AiWorkPhase::Executing);
    }

    #[test]
    fn registry_keeps_sessions_in_stable_registration_order() {
        let now = Instant::now();
        let mut registry = ClaudeSessionRegistry::new();
        registry
            .apply(hook("SessionStart", serde_json::json!({})), now)
            .unwrap();
        let second = hook_for_session("session-2", "SessionStart", serde_json::json!({}));
        registry.apply(second, now).unwrap();
        registry
            .apply(
                hook_for_session("session-1", "UserPromptSubmit", serde_json::json!({})),
                now,
            )
            .unwrap();
        let sessions = registry.snapshots();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "session-1");
        assert_eq!(sessions[1].session_id, "session-2");
    }

    #[test]
    fn registry_expires_completed_sessions_while_they_are_not_displayed() {
        let now = Instant::now();
        let mut registry = ClaudeSessionRegistry::new();
        registry
            .apply(hook("SessionStart", serde_json::json!({})), now)
            .unwrap();
        registry
            .apply(
                hook_for_session("session-2", "SessionStart", serde_json::json!({})),
                now,
            )
            .unwrap();
        registry
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        registry
            .apply(hook("Stop", serde_json::json!({})), now)
            .unwrap();

        let changes = registry.tick(now + CLAUDE_COMPLETED_DISPLAY_DURATION);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].state.session_id, "session-1");
        assert_eq!(changes[0].reason, ClaudeStateChangeReason::CompletedExpired);
        assert_eq!(changes[0].state.activity_state, AiActivityState::Available);
        let snapshots = registry.snapshots();
        assert_eq!(snapshots[0].activity_state, AiActivityState::Available);
        assert_eq!(snapshots[1].activity_state, AiActivityState::Available);
    }

    #[test]
    fn registry_retires_all_sessions_for_a_launch() {
        let now = Instant::now();
        let mut registry = ClaudeSessionRegistry::new();
        registry
            .apply(hook("SessionStart", serde_json::json!({})), now)
            .unwrap();
        registry
            .apply(
                hook_for_session("session-2", "SessionStart", serde_json::json!({})),
                now,
            )
            .unwrap();
        let changes = registry
            .apply(
                ClaudeObserverEvent::WrapperExited(ClaudeWrapperExited {
                    launch_id: "launch-1".to_string(),
                    exit_code: 0,
                }),
                now,
            )
            .unwrap();
        assert_eq!(changes.len(), 2);
        assert!(registry
            .snapshots()
            .iter()
            .all(|snapshot| !snapshot.session_active));
    }

    #[test]
    fn registry_keeps_independent_launches_as_distinct_sessions() {
        let now = Instant::now();
        let mut registry = ClaudeSessionRegistry::new();
        registry
            .apply(
                hook_for_launch_session(
                    "launch-1",
                    "session-1",
                    "SessionStart",
                    serde_json::json!({}),
                ),
                now,
            )
            .unwrap();
        registry
            .apply(
                hook_for_launch_session(
                    "launch-2",
                    "session-2",
                    "SessionStart",
                    serde_json::json!({}),
                ),
                now,
            )
            .unwrap();
        let sessions = registry.snapshots();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].launch_id, "launch-1");
        assert_eq!(sessions[1].launch_id, "launch-2");
    }

    #[test]
    fn registry_marks_only_the_overflowed_launch_desynchronized() {
        let now = Instant::now();
        let mut registry = ClaudeSessionRegistry::new();
        registry
            .apply(hook("SessionStart", serde_json::json!({})), now)
            .unwrap();
        registry
            .apply(hook("UserPromptSubmit", serde_json::json!({})), now)
            .unwrap();
        let changes = registry.mark_launch_desynchronized("launch-1");
        assert_eq!(changes.len(), 1);
        assert!(changes[0].state.desynchronized);
        assert!(registry
            .mark_launch_desynchronized("other-launch")
            .is_empty());
    }
}
