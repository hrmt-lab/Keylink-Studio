use std::{
    collections::{HashMap, VecDeque},
    sync::{mpsc, Arc, Mutex, RwLock},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    codex_broker::{
        BrokerDirection, CodexBrokerEvent, CodexBrokerManager, JsonRpcKind, JsonRpcMetadata,
    },
    packet::{AiActivityState, AiClientType, AiClientVariant, AiWorkPhase},
};

const RECONNECT_GRACE: Duration = Duration::from_secs(3);
const COMPLETED_DISPLAY_DURATION: Duration = Duration::from_secs(30);
const THINKING_STABILITY: Duration = Duration::from_millis(150);
const EXECUTION_RETURN_STABILITY: Duration = Duration::from_millis(250);
const MAX_PENDING_CHANGES: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiClientStateChangeReason {
    SessionStarted,
    SessionForked,
    SessionReplaced,
    SessionEnded,
    TurnStarted,
    TurnCompleted,
    CompletedExpired,
    TurnFailed,
    TurnInterrupted,
    RequestStarted,
    RequestResolved,
    WorkPhaseChanged,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct AiClientStateSnapshot {
    pub client_type: AiClientType,
    pub client_variant: AiClientVariant,
    pub session_active: bool,
    pub activity_state: AiActivityState,
    pub work_phase: AiWorkPhase,
    pub revision: u16,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct AiClientStateChange {
    pub state: AiClientStateSnapshot,
    pub reason: AiClientStateChangeReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestKind {
    Approval,
    Input,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
    InterruptedWithError,
}

#[derive(Debug)]
enum AiClientEvent {
    SessionRequested {
        requested_thread_id: Option<String>,
    },
    SessionStarted {
        thread_id: String,
    },
    SessionForked {
        thread_id: String,
    },
    TurnStarted {
        thread_id: String,
        turn_id: String,
    },
    RequestStarted {
        key: String,
        kind: RequestKind,
        thread_id: String,
        turn_id: Option<String>,
    },
    RequestResolved {
        key: String,
    },
    ItemStarted {
        thread_id: String,
        turn_id: String,
        item_id: String,
        work_phase: AiWorkPhase,
    },
    ItemCompleted {
        thread_id: String,
        turn_id: String,
        item_id: String,
    },
    TurnFinished {
        thread_id: String,
        turn_id: String,
        outcome: TurnOutcome,
    },
    ClientDisconnected,
    SessionEnded,
}

pub struct AiClientStateReducer {
    snapshot: AiClientStateSnapshot,
    has_emitted: bool,
    tracked_thread_id: Option<String>,
    tracked_turn_id: Option<String>,
    requests: HashMap<String, RequestKind>,
    active_items: HashMap<String, AiWorkPhase>,
    observed_work_phase: AiWorkPhase,
    pending_work_phase: Option<(AiWorkPhase, Instant)>,
    completed_deadline: Option<Instant>,
    reconnect_deadline: Option<Instant>,
}

impl AiClientStateReducer {
    pub fn new_codex_cli() -> Self {
        Self::with_initial_revision(initial_revision())
    }

    pub fn with_initial_revision(revision: u16) -> Self {
        Self {
            snapshot: AiClientStateSnapshot {
                client_type: AiClientType::Codex,
                client_variant: AiClientVariant::Cli,
                session_active: false,
                activity_state: AiActivityState::None,
                work_phase: AiWorkPhase::Unspecified,
                revision,
            },
            has_emitted: false,
            tracked_thread_id: None,
            tracked_turn_id: None,
            requests: HashMap::new(),
            active_items: HashMap::new(),
            observed_work_phase: AiWorkPhase::Unspecified,
            pending_work_phase: None,
            completed_deadline: None,
            reconnect_deadline: None,
        }
    }

    pub fn snapshot(&self) -> AiClientStateSnapshot {
        self.snapshot
    }

    fn apply(&mut self, event: AiClientEvent, now: Instant) -> Vec<AiClientStateChange> {
        match event {
            AiClientEvent::SessionRequested {
                requested_thread_id,
            } => {
                if !self.snapshot.session_active {
                    return Vec::new();
                }
                if requested_thread_id.as_deref() == self.tracked_thread_id.as_deref() {
                    return Vec::new();
                }
                self.clear_session();
                vec![self.emit(
                    false,
                    AiActivityState::None,
                    AiClientStateChangeReason::SessionReplaced,
                )]
            }
            AiClientEvent::SessionStarted { thread_id } => {
                if self.tracked_thread_id.as_deref() == Some(thread_id.as_str()) {
                    self.reconnect_deadline = None;
                    return Vec::new();
                }
                let mut changes = Vec::new();
                if self.snapshot.session_active {
                    self.clear_session();
                    changes.push(self.emit(
                        false,
                        AiActivityState::None,
                        AiClientStateChangeReason::SessionReplaced,
                    ));
                }
                self.tracked_thread_id = Some(thread_id);
                self.reconnect_deadline = None;
                changes.push(self.emit(
                    true,
                    AiActivityState::Available,
                    AiClientStateChangeReason::SessionStarted,
                ));
                changes
            }
            AiClientEvent::SessionForked { thread_id } => {
                if self.tracked_thread_id.as_deref() == Some(thread_id.as_str()) {
                    return Vec::new();
                }
                // A fork is a new active display thread, but it does not end the
                // parent CLI thread. Do not emit NONE between the two display
                // states; otherwise ScreenKey visibly blacks out before the forked
                // turn starts.
                self.clear_session();
                self.tracked_thread_id = Some(thread_id);
                vec![self.emit(
                    true,
                    AiActivityState::Available,
                    AiClientStateChangeReason::SessionForked,
                )]
            }
            AiClientEvent::TurnStarted { thread_id, turn_id } => {
                if self.tracked_thread_id.as_deref() != Some(thread_id.as_str()) {
                    return Vec::new();
                }
                self.tracked_turn_id = Some(turn_id);
                self.requests.clear();
                self.clear_items();
                self.completed_deadline = None;
                vec![self.emit(
                    true,
                    AiActivityState::Working,
                    AiClientStateChangeReason::TurnStarted,
                )]
            }
            AiClientEvent::RequestStarted {
                key,
                kind,
                thread_id,
                turn_id,
            } => {
                if self.tracked_thread_id.as_deref() != Some(thread_id.as_str()) {
                    return Vec::new();
                }
                if let Some(turn_id) = turn_id {
                    if self.tracked_turn_id.as_deref() != Some(turn_id.as_str()) {
                        return Vec::new();
                    }
                } else if kind == RequestKind::Approval {
                    return Vec::new();
                }
                if self.requests.insert(key, kind).is_some() {
                    return Vec::new();
                }
                vec![self.emit(
                    true,
                    self.waiting_state(),
                    AiClientStateChangeReason::RequestStarted,
                )]
            }
            AiClientEvent::RequestResolved { key } => {
                if self.requests.remove(&key).is_none() {
                    return Vec::new();
                }
                vec![self.emit(
                    true,
                    self.waiting_state(),
                    AiClientStateChangeReason::RequestResolved,
                )]
            }
            AiClientEvent::ItemStarted {
                thread_id,
                turn_id,
                item_id,
                work_phase,
            } => {
                if !self.matches_turn(&thread_id, &turn_id) {
                    return Vec::new();
                }
                if self.active_items.get(&item_id) == Some(&work_phase) {
                    return Vec::new();
                }
                self.active_items.insert(item_id, work_phase);
                self.update_observed_work_phase(now)
            }
            AiClientEvent::ItemCompleted {
                thread_id,
                turn_id,
                item_id,
            } => {
                if !self.matches_turn(&thread_id, &turn_id)
                    || self.active_items.remove(&item_id).is_none()
                {
                    return Vec::new();
                }
                self.update_observed_work_phase(now)
            }
            AiClientEvent::TurnFinished {
                thread_id,
                turn_id,
                outcome,
            } => {
                if self.tracked_thread_id.as_deref() != Some(thread_id.as_str())
                    || self.tracked_turn_id.as_deref() != Some(turn_id.as_str())
                {
                    return Vec::new();
                }
                self.tracked_turn_id = None;
                self.requests.clear();
                self.clear_items();
                self.completed_deadline = if outcome == TurnOutcome::Completed {
                    Some(now + COMPLETED_DISPLAY_DURATION)
                } else {
                    None
                };
                let (activity, reason) = match outcome {
                    TurnOutcome::Completed => (
                        AiActivityState::Completed,
                        AiClientStateChangeReason::TurnCompleted,
                    ),
                    TurnOutcome::Failed | TurnOutcome::InterruptedWithError => (
                        AiActivityState::Error,
                        AiClientStateChangeReason::TurnFailed,
                    ),
                    TurnOutcome::Interrupted => (
                        AiActivityState::Available,
                        AiClientStateChangeReason::TurnInterrupted,
                    ),
                };
                vec![self.emit(true, activity, reason)]
            }
            AiClientEvent::ClientDisconnected => {
                if self.snapshot.session_active {
                    self.reconnect_deadline = Some(now + RECONNECT_GRACE);
                }
                Vec::new()
            }
            AiClientEvent::SessionEnded => self.end_session(),
        }
    }

    fn tick(&mut self, now: Instant) -> Vec<AiClientStateChange> {
        if self
            .reconnect_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            return self.end_session();
        }
        if self
            .completed_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.completed_deadline = None;
            if self.snapshot.session_active
                && self.snapshot.activity_state == AiActivityState::Completed
            {
                return vec![self.emit(
                    true,
                    AiActivityState::Available,
                    AiClientStateChangeReason::CompletedExpired,
                )];
            }
        }
        if let Some((phase, deadline)) = self.pending_work_phase {
            if now >= deadline {
                self.pending_work_phase = None;
                if self.snapshot.activity_state == AiActivityState::Working
                    && self.observed_work_phase == phase
                    && self.snapshot.work_phase != phase
                {
                    return vec![self.emit_work_phase(phase)];
                }
            }
        }
        Vec::new()
    }

    fn end_session(&mut self) -> Vec<AiClientStateChange> {
        if !self.snapshot.session_active {
            self.clear_session();
            return Vec::new();
        }
        self.clear_session();
        vec![self.emit(
            false,
            AiActivityState::None,
            AiClientStateChangeReason::SessionEnded,
        )]
    }

    fn clear_session(&mut self) {
        self.tracked_thread_id = None;
        self.tracked_turn_id = None;
        self.requests.clear();
        self.clear_items();
        self.completed_deadline = None;
        self.reconnect_deadline = None;
    }

    fn waiting_state(&self) -> AiActivityState {
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
        } else if self.tracked_turn_id.is_some() {
            AiActivityState::Working
        } else {
            AiActivityState::Available
        }
    }

    fn matches_turn(&self, thread_id: &str, turn_id: &str) -> bool {
        self.tracked_thread_id.as_deref() == Some(thread_id)
            && self.tracked_turn_id.as_deref() == Some(turn_id)
    }

    fn clear_items(&mut self) {
        self.active_items.clear();
        self.observed_work_phase = AiWorkPhase::Unspecified;
        self.pending_work_phase = None;
    }

    fn aggregate_work_phase(&self) -> AiWorkPhase {
        self.active_items
            .values()
            .copied()
            .max_by_key(|phase| match phase {
                AiWorkPhase::Unspecified => 0,
                AiWorkPhase::Thinking => 1,
                AiWorkPhase::Executing => 2,
                AiWorkPhase::Searching => 3,
            })
            .unwrap_or(AiWorkPhase::Unspecified)
    }

    fn update_observed_work_phase(&mut self, now: Instant) -> Vec<AiClientStateChange> {
        let next = self.aggregate_work_phase();
        if next == self.observed_work_phase {
            return Vec::new();
        }
        self.observed_work_phase = next;
        if self.snapshot.activity_state != AiActivityState::Working {
            self.pending_work_phase = None;
            return Vec::new();
        }
        if next == self.snapshot.work_phase {
            self.pending_work_phase = None;
            return Vec::new();
        }
        if matches!(next, AiWorkPhase::Executing | AiWorkPhase::Searching) {
            self.pending_work_phase = None;
            return vec![self.emit_work_phase(next)];
        }
        let delay = if matches!(
            self.snapshot.work_phase,
            AiWorkPhase::Executing | AiWorkPhase::Searching
        ) {
            EXECUTION_RETURN_STABILITY
        } else {
            THINKING_STABILITY
        };
        self.pending_work_phase = Some((next, now + delay));
        Vec::new()
    }

    fn emit_work_phase(&mut self, work_phase: AiWorkPhase) -> AiClientStateChange {
        self.snapshot.work_phase = work_phase;
        AiClientStateChange {
            state: self.snapshot,
            reason: AiClientStateChangeReason::WorkPhaseChanged,
        }
    }

    fn emit(
        &mut self,
        session_active: bool,
        activity_state: AiActivityState,
        reason: AiClientStateChangeReason,
    ) -> AiClientStateChange {
        if self.has_emitted {
            self.snapshot.revision = self.snapshot.revision.wrapping_add(1);
        } else {
            self.has_emitted = true;
        }
        self.snapshot.session_active = session_active;
        self.snapshot.activity_state = activity_state;
        self.pending_work_phase = None;
        self.snapshot.work_phase = if activity_state == AiActivityState::Working {
            self.observed_work_phase
        } else {
            AiWorkPhase::Unspecified
        };
        AiClientStateChange {
            state: self.snapshot,
            reason,
        }
    }
}

impl Default for AiClientStateReducer {
    fn default() -> Self {
        Self::new_codex_cli()
    }
}

#[derive(Debug)]
enum PendingClientRequest {
    ThreadStart,
    ThreadResume { requested_thread_id: String },
    ThreadFork,
}

#[derive(Debug)]
struct PendingServerRequest {
    thread_id: String,
    turn_id: Option<String>,
}

#[derive(Debug, Default)]
pub struct CodexEventAdapter {
    connection_id: Option<String>,
    confirmed_thread_id: Option<String>,
    announced_thread_id: Option<String>,
    client_requests: HashMap<String, PendingClientRequest>,
    server_requests: HashMap<String, PendingServerRequest>,
}

impl CodexEventAdapter {
    fn adapt(&mut self, event: CodexBrokerEvent) -> Vec<AiClientEvent> {
        match event {
            CodexBrokerEvent::ClientConnected { connection_id } => {
                self.connection_id = Some(connection_id);
                self.announced_thread_id = None;
                self.client_requests.clear();
                self.server_requests.clear();
                Vec::new()
            }
            CodexBrokerEvent::ClientDisconnected {
                connection_id,
                origin,
            } if self.connection_id.as_deref() == Some(connection_id.as_str()) => {
                self.connection_id = None;
                self.announced_thread_id = None;
                self.client_requests.clear();
                self.server_requests.clear();
                if origin == "cli" {
                    vec![AiClientEvent::ClientDisconnected]
                } else {
                    vec![AiClientEvent::SessionEnded]
                }
            }
            CodexBrokerEvent::Message {
                connection_id,
                direction,
                metadata,
            } if self.connection_id.as_deref() == Some(connection_id.as_str()) => {
                self.adapt_message(direction, *metadata)
            }
            CodexBrokerEvent::Stopped | CodexBrokerEvent::Error { .. } => {
                self.connection_id = None;
                self.confirmed_thread_id = None;
                self.announced_thread_id = None;
                self.client_requests.clear();
                self.server_requests.clear();
                vec![AiClientEvent::SessionEnded]
            }
            _ => Vec::new(),
        }
    }

    fn adapt_message(
        &mut self,
        direction: BrokerDirection,
        metadata: JsonRpcMetadata,
    ) -> Vec<AiClientEvent> {
        match direction {
            BrokerDirection::CliToAppServer => self.adapt_cli_message(metadata),
            BrokerDirection::AppServerToCli => self.adapt_server_message(metadata),
        }
    }

    fn adapt_cli_message(&mut self, metadata: JsonRpcMetadata) -> Vec<AiClientEvent> {
        if metadata.kind == JsonRpcKind::Request {
            let Some(key) = metadata.id.as_ref().and_then(rpc_key) else {
                return Vec::new();
            };
            match metadata.method.as_deref() {
                Some("thread/start") => {
                    self.confirmed_thread_id = None;
                    self.announced_thread_id = None;
                    self.server_requests.clear();
                    self.client_requests
                        .insert(key, PendingClientRequest::ThreadStart);
                    return vec![AiClientEvent::SessionRequested {
                        requested_thread_id: None,
                    }];
                }
                Some("thread/resume") => {
                    if let Some(thread_id) = metadata.thread_id {
                        self.announced_thread_id = None;
                        self.server_requests.clear();
                        self.client_requests.insert(
                            key,
                            PendingClientRequest::ThreadResume {
                                requested_thread_id: thread_id.clone(),
                            },
                        );
                        return vec![AiClientEvent::SessionRequested {
                            requested_thread_id: Some(thread_id),
                        }];
                    }
                }
                Some("thread/fork") => {
                    self.client_requests
                        .insert(key, PendingClientRequest::ThreadFork);
                }
                _ => {}
            }
            return Vec::new();
        }
        if metadata.kind != JsonRpcKind::Response {
            return Vec::new();
        }
        let Some(key) = metadata.id.as_ref().and_then(rpc_key) else {
            return Vec::new();
        };
        if self.server_requests.remove(&key).is_some() {
            return vec![AiClientEvent::RequestResolved { key }];
        }
        self.handle_thread_response(key, metadata)
    }

    fn handle_thread_response(
        &mut self,
        key: String,
        metadata: JsonRpcMetadata,
    ) -> Vec<AiClientEvent> {
        let Some(request) = self.client_requests.remove(&key) else {
            return Vec::new();
        };
        let Some(result_thread_id) = metadata.result_thread_id else {
            return Vec::new();
        };
        if self
            .announced_thread_id
            .as_deref()
            .is_some_and(|announced| announced != result_thread_id)
        {
            self.confirmed_thread_id = None;
            self.announced_thread_id = None;
            return vec![AiClientEvent::SessionEnded];
        }
        match request {
            PendingClientRequest::ThreadStart => {
                self.confirmed_thread_id = Some(result_thread_id.clone());
                vec![AiClientEvent::SessionStarted {
                    thread_id: result_thread_id,
                }]
            }
            PendingClientRequest::ThreadResume {
                requested_thread_id,
            } if requested_thread_id == result_thread_id => {
                self.confirmed_thread_id = Some(result_thread_id.clone());
                vec![AiClientEvent::SessionStarted {
                    thread_id: result_thread_id,
                }]
            }
            PendingClientRequest::ThreadResume { .. } => {
                self.confirmed_thread_id = None;
                vec![AiClientEvent::SessionEnded]
            }
            PendingClientRequest::ThreadFork => {
                self.confirmed_thread_id = Some(result_thread_id.clone());
                self.announced_thread_id = None;
                vec![AiClientEvent::SessionForked {
                    thread_id: result_thread_id,
                }]
            }
        }
    }

    fn adapt_server_message(&mut self, metadata: JsonRpcMetadata) -> Vec<AiClientEvent> {
        if metadata.kind == JsonRpcKind::Response {
            let Some(key) = metadata.id.as_ref().and_then(rpc_key) else {
                return Vec::new();
            };
            return self.handle_thread_response(key, metadata);
        }
        match (metadata.kind, metadata.method.as_deref()) {
            (JsonRpcKind::Notification, Some("thread/started")) => {
                let Some(thread_id) = metadata.thread_id else {
                    return Vec::new();
                };
                if let Some(confirmed) = self.confirmed_thread_id.as_deref() {
                    if confirmed != thread_id {
                        // `/side` (`/btw`) creates another, ephemeral thread while
                        // the parent remains active. Its notification must not end
                        // the ScreenKey session tracked for that parent. Explicit
                        // `thread/start` / `thread/resume` responses are the only
                        // session-replacement boundary.
                        return Vec::new();
                    }
                } else {
                    self.announced_thread_id = Some(thread_id);
                }
                Vec::new()
            }
            (JsonRpcKind::Notification, Some("turn/started"))
                if metadata.turn_status.as_deref() == Some("inProgress") =>
            {
                self.server_requests.clear();
                match (metadata.thread_id, metadata.turn_id) {
                    (Some(thread_id), Some(turn_id)) => {
                        // `/side` can return to its parent without issuing a
                        // `thread/resume` request. The next turn on either the
                        // parent or fork is therefore the authoritative display
                        // focus. Switch without emitting NONE, then apply the turn.
                        let mut events = Vec::new();
                        if self.confirmed_thread_id.as_deref() != Some(thread_id.as_str()) {
                            self.confirmed_thread_id = Some(thread_id.clone());
                            events.push(AiClientEvent::SessionForked {
                                thread_id: thread_id.clone(),
                            });
                        }
                        events.push(AiClientEvent::TurnStarted { thread_id, turn_id });
                        events
                    }
                    _ => Vec::new(),
                }
            }
            (JsonRpcKind::Notification, Some("turn/completed")) => {
                let outcome = match metadata.turn_status.as_deref() {
                    Some("completed") => Some(TurnOutcome::Completed),
                    Some("failed") => Some(TurnOutcome::Failed),
                    Some("interrupted") if metadata.turn_has_error => {
                        Some(TurnOutcome::InterruptedWithError)
                    }
                    Some("interrupted") => Some(TurnOutcome::Interrupted),
                    _ => None,
                };
                match (metadata.thread_id, metadata.turn_id, outcome) {
                    (Some(thread_id), Some(turn_id), Some(outcome)) => {
                        self.server_requests.retain(|_, request| {
                            request.thread_id != thread_id
                                || request.turn_id.as_deref() != Some(turn_id.as_str())
                        });
                        vec![AiClientEvent::TurnFinished {
                            thread_id,
                            turn_id,
                            outcome,
                        }]
                    }
                    _ => Vec::new(),
                }
            }
            (JsonRpcKind::Notification, Some("item/started")) => {
                let (Some(thread_id), Some(turn_id), Some(item_id), Some(work_phase)) = (
                    metadata.thread_id,
                    metadata.turn_id,
                    metadata.item_id,
                    metadata.item_type.as_deref().and_then(item_work_phase),
                ) else {
                    return Vec::new();
                };
                vec![AiClientEvent::ItemStarted {
                    thread_id,
                    turn_id,
                    item_id,
                    work_phase,
                }]
            }
            (JsonRpcKind::Notification, Some("item/completed")) => {
                let (Some(thread_id), Some(turn_id), Some(item_id)) =
                    (metadata.thread_id, metadata.turn_id, metadata.item_id)
                else {
                    return Vec::new();
                };
                vec![AiClientEvent::ItemCompleted {
                    thread_id,
                    turn_id,
                    item_id,
                }]
            }
            (JsonRpcKind::Request, Some(method)) => {
                let kind = if is_approval_method(method) {
                    Some(RequestKind::Approval)
                } else if is_input_method(method) {
                    Some(RequestKind::Input)
                } else {
                    None
                };
                let (Some(kind), Some(key), Some(thread_id)) = (
                    kind,
                    metadata.id.as_ref().and_then(rpc_key),
                    metadata.thread_id,
                ) else {
                    return Vec::new();
                };
                if method == "item/tool/requestUserInput" && metadata.turn_id.is_none() {
                    return Vec::new();
                }
                if method.starts_with("item/")
                    && (metadata.turn_id.is_none() || metadata.item_id.is_none())
                {
                    return Vec::new();
                }
                self.server_requests.insert(
                    key.clone(),
                    PendingServerRequest {
                        thread_id: thread_id.clone(),
                        turn_id: metadata.turn_id.clone(),
                    },
                );
                vec![AiClientEvent::RequestStarted {
                    key,
                    kind,
                    thread_id,
                    turn_id: metadata.turn_id,
                }]
            }
            (JsonRpcKind::Notification, Some("serverRequest/resolved")) => {
                let Some(key) = metadata.request_id.as_ref().and_then(rpc_key) else {
                    return Vec::new();
                };
                self.server_requests.remove(&key);
                vec![AiClientEvent::RequestResolved { key }]
            }
            // `error` notifications remain diagnostic metadata. They do not end a Turn.
            _ => Vec::new(),
        }
    }
}

pub struct CodexActivityRuntime {
    snapshot: Arc<RwLock<AiClientStateSnapshot>>,
    changes: Arc<Mutex<VecDeque<AiClientStateChange>>>,
    stop_tx: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
}

impl CodexActivityRuntime {
    pub fn start(broker: CodexBrokerManager) -> Self {
        let reducer = AiClientStateReducer::new_codex_cli();
        let snapshot = Arc::new(RwLock::new(reducer.snapshot()));
        let worker_snapshot = snapshot.clone();
        let changes = Arc::new(Mutex::new(VecDeque::new()));
        let worker_changes = changes.clone();
        let (stop_tx, stop_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("codex-activity-reducer".to_string())
            .spawn(move || {
                run_activity_loop(broker, reducer, worker_snapshot, worker_changes, stop_rx)
            })
            .expect("failed to create Codex Activity reducer thread");
        Self {
            snapshot,
            changes,
            stop_tx,
            worker: Some(worker),
        }
    }

    pub fn snapshot(&self) -> AiClientStateSnapshot {
        *self.snapshot.read().unwrap()
    }

    pub fn try_recv_change(&self) -> Option<AiClientStateChange> {
        self.changes.lock().unwrap().pop_front()
    }
}

impl Drop for CodexActivityRuntime {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn run_activity_loop(
    broker: CodexBrokerManager,
    mut reducer: AiClientStateReducer,
    snapshot: Arc<RwLock<AiClientStateSnapshot>>,
    changes: Arc<Mutex<VecDeque<AiClientStateChange>>>,
    stop_rx: mpsc::Receiver<()>,
) {
    let mut adapter = CodexEventAdapter::default();
    loop {
        if stop_rx.try_recv().is_ok() {
            break;
        }
        let now = Instant::now();
        publish_changes(reducer.tick(now), &snapshot, &changes);
        match broker.recv_event_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                let now = Instant::now();
                publish_changes(reducer.tick(now), &snapshot, &changes);
                for semantic_event in adapter.adapt(event) {
                    publish_changes(reducer.apply(semantic_event, now), &snapshot, &changes);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                publish_changes(
                    reducer.apply(AiClientEvent::SessionEnded, Instant::now()),
                    &snapshot,
                    &changes,
                );
                break;
            }
        }
    }
}

fn publish_changes(
    changes: Vec<AiClientStateChange>,
    snapshot: &Arc<RwLock<AiClientStateSnapshot>>,
    pending: &Arc<Mutex<VecDeque<AiClientStateChange>>>,
) {
    for change in changes {
        *snapshot.write().unwrap() = change.state;
        let mut pending = pending.lock().unwrap();
        if pending.len() == MAX_PENDING_CHANGES {
            pending.pop_front();
        }
        pending.push_back(change);
    }
}

fn is_approval_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
    )
}

fn is_input_method(method: &str) -> bool {
    matches!(
        method,
        "item/tool/requestUserInput" | "mcpServer/elicitation/request"
    )
}

fn item_work_phase(item_type: &str) -> Option<AiWorkPhase> {
    match item_type {
        "reasoning" | "agentMessage" | "plan" => Some(AiWorkPhase::Thinking),
        "commandExecution"
        | "fileChange"
        | "mcpToolCall"
        | "dynamicToolCall"
        | "collabAgentToolCall"
        | "subAgentActivity"
        | "imageView"
        | "imageGeneration"
        | "sleep" => Some(AiWorkPhase::Executing),
        "webSearch" => Some(AiWorkPhase::Searching),
        _ => None,
    }
}

fn rpc_key(value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("null".to_string()),
        Value::String(value) => Some(format!("s:{value}")),
        Value::Number(value) => Some(format!("n:{value}")),
        _ => None,
    }
}

fn initial_revision() -> u16 {
    let mut bytes = [0_u8; 2];
    if getrandom::fill(&mut bytes).is_ok() {
        return u16::from_le_bytes(bytes);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or_default();
    (nanos ^ u64::from(std::process::id())) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD_A: &str = "thread-a";
    const THREAD_B: &str = "thread-b";
    const TURN_A: &str = "turn-a";

    fn apply_one(
        reducer: &mut AiClientStateReducer,
        event: AiClientEvent,
        now: Instant,
    ) -> AiClientStateChange {
        let changes = reducer.apply(event, now);
        assert_eq!(changes.len(), 1);
        changes[0]
    }

    fn start_session(reducer: &mut AiClientStateReducer, now: Instant) -> AiClientStateChange {
        apply_one(
            reducer,
            AiClientEvent::SessionStarted {
                thread_id: THREAD_A.to_string(),
            },
            now,
        )
    }

    fn start_turn(reducer: &mut AiClientStateReducer, now: Instant) -> AiClientStateChange {
        apply_one(
            reducer,
            AiClientEvent::TurnStarted {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
            },
            now,
        )
    }

    fn message(direction: BrokerDirection, json: &str) -> CodexBrokerEvent {
        CodexBrokerEvent::Message {
            connection_id: "connection-1".to_string(),
            direction,
            metadata: Box::new(crate::codex_broker::classify_json_rpc(json)),
        }
    }

    #[test]
    fn first_state_uses_initial_revision_and_subsequent_states_wrap() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(u16::MAX);

        let session = start_session(&mut reducer, now);
        assert_eq!(session.reason, AiClientStateChangeReason::SessionStarted);
        assert_eq!(session.state.revision, u16::MAX);
        assert_eq!(session.state.activity_state, AiActivityState::Available);

        let turn = start_turn(&mut reducer, now);
        assert_eq!(turn.state.revision, 0);
        assert_eq!(turn.state.activity_state, AiActivityState::Working);
    }

    #[test]
    fn approval_has_priority_over_input_and_resolution_restores_prior_state() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(10);
        start_session(&mut reducer, now);
        start_turn(&mut reducer, now);

        let input = apply_one(
            &mut reducer,
            AiClientEvent::RequestStarted {
                key: "input".to_string(),
                kind: RequestKind::Input,
                thread_id: THREAD_A.to_string(),
                turn_id: Some(TURN_A.to_string()),
            },
            now,
        );
        assert_eq!(input.state.activity_state, AiActivityState::WaitingInput);

        let approval = apply_one(
            &mut reducer,
            AiClientEvent::RequestStarted {
                key: "approval".to_string(),
                kind: RequestKind::Approval,
                thread_id: THREAD_A.to_string(),
                turn_id: Some(TURN_A.to_string()),
            },
            now,
        );
        assert_eq!(
            approval.state.activity_state,
            AiActivityState::WaitingApproval
        );

        let after_approval = apply_one(
            &mut reducer,
            AiClientEvent::RequestResolved {
                key: "approval".to_string(),
            },
            now,
        );
        assert_eq!(
            after_approval.state.activity_state,
            AiActivityState::WaitingInput
        );

        let after_input = apply_one(
            &mut reducer,
            AiClientEvent::RequestResolved {
                key: "input".to_string(),
            },
            now,
        );
        assert_eq!(after_input.state.activity_state, AiActivityState::Working);
    }

    #[test]
    fn completed_failed_and_interrupted_turns_follow_the_contract() {
        let now = Instant::now();
        let outcomes = [
            (TurnOutcome::Completed, AiActivityState::Completed),
            (TurnOutcome::Failed, AiActivityState::Error),
            (TurnOutcome::InterruptedWithError, AiActivityState::Error),
            (TurnOutcome::Interrupted, AiActivityState::Available),
        ];

        for (outcome, expected) in outcomes {
            let mut reducer = AiClientStateReducer::with_initial_revision(1);
            start_session(&mut reducer, now);
            start_turn(&mut reducer, now);
            let change = apply_one(
                &mut reducer,
                AiClientEvent::TurnFinished {
                    thread_id: THREAD_A.to_string(),
                    turn_id: TURN_A.to_string(),
                    outcome,
                },
                now,
            );
            assert_eq!(change.state.activity_state, expected);
            assert!(change.state.session_active);
        }
    }

    #[test]
    fn completed_state_expires_to_available_after_thirty_seconds() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(70);
        start_session(&mut reducer, now);
        start_turn(&mut reducer, now);
        let completed = apply_one(
            &mut reducer,
            AiClientEvent::TurnFinished {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
                outcome: TurnOutcome::Completed,
            },
            now,
        );

        assert_eq!(completed.state.activity_state, AiActivityState::Completed);
        assert!(reducer
            .tick(now + COMPLETED_DISPLAY_DURATION - Duration::from_millis(1))
            .is_empty());

        let expired = reducer.tick(now + COMPLETED_DISPLAY_DURATION);
        assert_eq!(expired.len(), 1);
        assert_eq!(
            expired[0].reason,
            AiClientStateChangeReason::CompletedExpired
        );
        assert_eq!(expired[0].state.activity_state, AiActivityState::Available);
        assert_eq!(expired[0].state.work_phase, AiWorkPhase::Unspecified);
        assert_eq!(
            expired[0].state.revision,
            completed.state.revision.wrapping_add(1)
        );
        assert!(reducer
            .tick(now + COMPLETED_DISPLAY_DURATION + Duration::from_secs(1))
            .is_empty());
    }

    #[test]
    fn starting_a_new_turn_cancels_completed_expiration() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(80);
        start_session(&mut reducer, now);
        start_turn(&mut reducer, now);
        apply_one(
            &mut reducer,
            AiClientEvent::TurnFinished {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
                outcome: TurnOutcome::Completed,
            },
            now,
        );
        apply_one(
            &mut reducer,
            AiClientEvent::TurnStarted {
                thread_id: THREAD_A.to_string(),
                turn_id: "turn-b".to_string(),
            },
            now + Duration::from_secs(1),
        );

        assert!(reducer.tick(now + COMPLETED_DISPLAY_DURATION).is_empty());
        assert_eq!(reducer.snapshot().activity_state, AiActivityState::Working);
    }

    #[test]
    fn reconnecting_same_thread_preserves_state_and_revision_until_grace_expires() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(100);
        start_session(&mut reducer, now);
        let working = start_turn(&mut reducer, now);

        assert!(reducer
            .apply(AiClientEvent::ClientDisconnected, now)
            .is_empty());
        assert!(reducer
            .apply(
                AiClientEvent::SessionStarted {
                    thread_id: THREAD_A.to_string(),
                },
                now + Duration::from_secs(1),
            )
            .is_empty());
        assert_eq!(reducer.snapshot(), working.state);

        assert!(reducer
            .apply(
                AiClientEvent::ClientDisconnected,
                now + Duration::from_secs(2)
            )
            .is_empty());
        let changes = reducer.tick(now + Duration::from_secs(6));
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].state.activity_state, AiActivityState::None);
        assert!(!changes[0].state.session_active);
        assert_eq!(
            changes[0].state.revision,
            working.state.revision.wrapping_add(1)
        );
    }

    #[test]
    fn replacing_a_session_emits_none_before_new_thread_becomes_available() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(1);
        start_session(&mut reducer, now);

        let ended = apply_one(
            &mut reducer,
            AiClientEvent::SessionRequested {
                requested_thread_id: Some(THREAD_B.to_string()),
            },
            now,
        );
        assert_eq!(ended.reason, AiClientStateChangeReason::SessionReplaced);
        assert_eq!(ended.state.activity_state, AiActivityState::None);

        let new_session = apply_one(
            &mut reducer,
            AiClientEvent::SessionStarted {
                thread_id: THREAD_B.to_string(),
            },
            now,
        );
        assert_eq!(new_session.state.activity_state, AiActivityState::Available);
        assert!(new_session.state.session_active);
    }

    #[test]
    fn item_types_map_without_inspecting_item_content() {
        for item_type in ["reasoning", "agentMessage", "plan"] {
            assert_eq!(item_work_phase(item_type), Some(AiWorkPhase::Thinking));
        }
        for item_type in [
            "commandExecution",
            "fileChange",
            "mcpToolCall",
            "dynamicToolCall",
            "collabAgentToolCall",
            "subAgentActivity",
            "imageView",
            "imageGeneration",
            "sleep",
        ] {
            assert_eq!(item_work_phase(item_type), Some(AiWorkPhase::Executing));
        }
        assert_eq!(item_work_phase("webSearch"), Some(AiWorkPhase::Searching));
        assert_eq!(item_work_phase("userMessage"), None);
        assert_eq!(item_work_phase("futureItemType"), None);
    }

    #[test]
    fn adapter_emits_structured_item_lifecycle_events() {
        let mut adapter = CodexEventAdapter::default();
        adapter.adapt(CodexBrokerEvent::ClientConnected {
            connection_id: "connection-1".to_string(),
        });

        let started = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"item-a","type":"webSearch","query":"must-not-be-inspected"}}}"#,
        ));
        assert!(matches!(
            started.as_slice(),
            [AiClientEvent::ItemStarted {
                thread_id,
                turn_id,
                item_id,
                work_phase: AiWorkPhase::Searching,
            }] if thread_id == THREAD_A && turn_id == TURN_A && item_id == "item-a"
        ));

        let completed = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"item-a","type":"webSearch"}}}"#,
        ));
        assert!(matches!(
            completed.as_slice(),
            [AiClientEvent::ItemCompleted {
                thread_id,
                turn_id,
                item_id,
            }] if thread_id == THREAD_A && turn_id == TURN_A && item_id == "item-a"
        ));
    }

    #[test]
    fn work_phase_precedence_and_debounce_do_not_change_base_revision() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(20);
        start_session(&mut reducer, now);
        let working = start_turn(&mut reducer, now);

        assert!(reducer
            .apply(
                AiClientEvent::ItemStarted {
                    thread_id: THREAD_A.to_string(),
                    turn_id: TURN_A.to_string(),
                    item_id: "thinking".to_string(),
                    work_phase: AiWorkPhase::Thinking,
                },
                now,
            )
            .is_empty());
        let thinking = reducer.tick(now + THINKING_STABILITY);
        assert_eq!(thinking.len(), 1);
        assert_eq!(thinking[0].state.work_phase, AiWorkPhase::Thinking);
        assert_eq!(thinking[0].state.revision, working.state.revision);

        let executing = apply_one(
            &mut reducer,
            AiClientEvent::ItemStarted {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
                item_id: "tool".to_string(),
                work_phase: AiWorkPhase::Executing,
            },
            now + THINKING_STABILITY,
        );
        assert_eq!(executing.state.work_phase, AiWorkPhase::Executing);
        assert_eq!(executing.state.revision, working.state.revision);

        let searching = apply_one(
            &mut reducer,
            AiClientEvent::ItemStarted {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
                item_id: "search".to_string(),
                work_phase: AiWorkPhase::Searching,
            },
            now + THINKING_STABILITY,
        );
        assert_eq!(searching.state.work_phase, AiWorkPhase::Searching);

        let back_to_executing = apply_one(
            &mut reducer,
            AiClientEvent::ItemCompleted {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
                item_id: "search".to_string(),
            },
            now + THINKING_STABILITY,
        );
        assert_eq!(back_to_executing.state.work_phase, AiWorkPhase::Executing);
        assert!(reducer
            .apply(
                AiClientEvent::ItemCompleted {
                    thread_id: THREAD_A.to_string(),
                    turn_id: TURN_A.to_string(),
                    item_id: "tool".to_string(),
                },
                now + THINKING_STABILITY,
            )
            .is_empty());
        assert!(reducer
            .tick(now + THINKING_STABILITY + Duration::from_millis(249))
            .is_empty());
        let returned = reducer.tick(now + THINKING_STABILITY + EXECUTION_RETURN_STABILITY);
        assert_eq!(returned.len(), 1);
        assert_eq!(returned[0].state.work_phase, AiWorkPhase::Thinking);
        assert_eq!(returned[0].state.revision, working.state.revision);
    }

    #[test]
    fn waiting_state_hides_phase_and_resolution_restores_active_phase_immediately() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(30);
        start_session(&mut reducer, now);
        start_turn(&mut reducer, now);
        apply_one(
            &mut reducer,
            AiClientEvent::ItemStarted {
                thread_id: THREAD_A.to_string(),
                turn_id: TURN_A.to_string(),
                item_id: "tool".to_string(),
                work_phase: AiWorkPhase::Executing,
            },
            now,
        );

        let waiting = apply_one(
            &mut reducer,
            AiClientEvent::RequestStarted {
                key: "approval".to_string(),
                kind: RequestKind::Approval,
                thread_id: THREAD_A.to_string(),
                turn_id: Some(TURN_A.to_string()),
            },
            now,
        );
        assert_eq!(
            waiting.state.activity_state,
            AiActivityState::WaitingApproval
        );
        assert_eq!(waiting.state.work_phase, AiWorkPhase::Unspecified);

        let restored = apply_one(
            &mut reducer,
            AiClientEvent::RequestResolved {
                key: "approval".to_string(),
            },
            now,
        );
        assert_eq!(restored.state.activity_state, AiActivityState::Working);
        assert_eq!(restored.state.work_phase, AiWorkPhase::Executing);
    }

    #[test]
    fn item_events_for_other_turns_and_unknown_completions_are_ignored() {
        let now = Instant::now();
        let mut reducer = AiClientStateReducer::with_initial_revision(40);
        start_session(&mut reducer, now);
        start_turn(&mut reducer, now);

        assert!(reducer
            .apply(
                AiClientEvent::ItemStarted {
                    thread_id: THREAD_A.to_string(),
                    turn_id: "other-turn".to_string(),
                    item_id: "tool".to_string(),
                    work_phase: AiWorkPhase::Executing,
                },
                now,
            )
            .is_empty());
        assert!(reducer
            .apply(
                AiClientEvent::ItemCompleted {
                    thread_id: THREAD_A.to_string(),
                    turn_id: TURN_A.to_string(),
                    item_id: "missing".to_string(),
                },
                now,
            )
            .is_empty());
        assert_eq!(reducer.snapshot().work_phase, AiWorkPhase::Unspecified);
    }

    #[test]
    fn adapter_correlates_thread_turn_and_response_required_requests() {
        let mut adapter = CodexEventAdapter::default();
        assert!(adapter
            .adapt(CodexBrokerEvent::ClientConnected {
                connection_id: "connection-1".to_string(),
            })
            .is_empty());

        let events = adapter.adapt(message(
            BrokerDirection::CliToAppServer,
            r#"{"jsonrpc":"2.0","id":1,"method":"thread/start","params":{}}"#,
        ));
        assert!(matches!(
            events.as_slice(),
            [AiClientEvent::SessionRequested {
                requested_thread_id: None
            }]
        ));

        let events = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","id":1,"result":{"thread":{"id":"thread-a"}}}"#,
        ));
        assert!(matches!(
            events.as_slice(),
            [AiClientEvent::SessionStarted { thread_id }] if thread_id == THREAD_A
        ));

        assert!(adapter
            .adapt(message(
                BrokerDirection::AppServerToCli,
                r#"{"jsonrpc":"2.0","method":"thread/started","params":{"thread":{"id":"thread-a"}}}"#,
            ))
            .is_empty());

        let events = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"inProgress"}}}"#,
        ));
        assert!(matches!(
            events.as_slice(),
            [AiClientEvent::TurnStarted { thread_id, turn_id }]
                if thread_id == THREAD_A && turn_id == TURN_A
        ));

        let events = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","id":99,"method":"item/commandExecution/requestApproval","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"item-a"}}}"#,
        ));
        assert!(matches!(
            events.as_slice(),
            [AiClientEvent::RequestStarted {
                key,
                kind: RequestKind::Approval,
                thread_id,
                turn_id,
            }] if key == "n:99"
                && thread_id == THREAD_A
                && turn_id.as_deref() == Some(TURN_A)
        ));

        let events = adapter.adapt(message(
            BrokerDirection::CliToAppServer,
            r#"{"jsonrpc":"2.0","id":99,"result":{}}"#,
        ));
        assert!(matches!(
            events.as_slice(),
            [AiClientEvent::RequestResolved { key }] if key == "n:99"
        ));
    }

    #[test]
    fn adapter_ignores_a_side_thread_started_notification() {
        let mut adapter = CodexEventAdapter::default();
        adapter.adapt(CodexBrokerEvent::ClientConnected {
            connection_id: "connection-1".to_string(),
        });
        adapter.adapt(message(
            BrokerDirection::CliToAppServer,
            r#"{"jsonrpc":"2.0","id":"start","method":"thread/start","params":{}}"#,
        ));
        adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","id":"start","result":{"thread":{"id":"thread-a"}}}"#,
        ));

        let events = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"thread/started","params":{"thread":{"id":"thread-b"}}}"#,
        ));
        assert!(events.is_empty());
        assert_eq!(adapter.confirmed_thread_id.as_deref(), Some(THREAD_A));

        let events = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-a","status":"inProgress"}}}"#,
        ));
        assert!(matches!(
            events.as_slice(),
            [AiClientEvent::TurnStarted {
                thread_id,
                turn_id,
            }] if thread_id == THREAD_A && turn_id == TURN_A
        ));
    }

    #[test]
    fn adapter_promotes_a_forked_thread_without_blacking_out_screenkey() {
        let now = Instant::now();
        let mut adapter = CodexEventAdapter::default();
        let mut reducer = AiClientStateReducer::with_initial_revision(90);
        start_session(&mut reducer, now);
        adapter.adapt(CodexBrokerEvent::ClientConnected {
            connection_id: "connection-1".to_string(),
        });
        adapter.adapt(message(
            BrokerDirection::CliToAppServer,
            r#"{"jsonrpc":"2.0","id":"start","method":"thread/start","params":{}}"#,
        ));
        adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","id":"start","result":{"thread":{"id":"thread-a"}}}"#,
        ));

        assert!(adapter
            .adapt(message(
                BrokerDirection::CliToAppServer,
                r#"{"jsonrpc":"2.0","id":"fork","method":"thread/fork","params":{"threadId":"thread-a","ephemeral":true}}"#,
            ))
            .is_empty());
        let forked = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","id":"fork","result":{"thread":{"id":"thread-b","forkedFromId":"thread-a"}}}"#,
        ));
        assert!(matches!(
            forked.as_slice(),
            [AiClientEvent::SessionForked { thread_id }] if thread_id == THREAD_B
        ));
        let switched = apply_one(&mut reducer, forked.into_iter().next().unwrap(), now);
        assert_eq!(switched.reason, AiClientStateChangeReason::SessionForked);
        assert!(switched.state.session_active);
        assert_eq!(switched.state.activity_state, AiActivityState::Available);

        assert!(adapter
            .adapt(message(
                BrokerDirection::AppServerToCli,
                r#"{"jsonrpc":"2.0","method":"thread/started","params":{"thread":{"id":"thread-b"}}}"#,
            ))
            .is_empty());
        let started = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"thread-b","turn":{"id":"turn-b","status":"inProgress"}}}"#,
        ));
        let working = apply_one(&mut reducer, started.into_iter().next().unwrap(), now);
        assert_eq!(working.state.activity_state, AiActivityState::Working);

        let completed = apply_one(
            &mut reducer,
            AiClientEvent::TurnFinished {
                thread_id: THREAD_B.to_string(),
                turn_id: "turn-b".to_string(),
                outcome: TurnOutcome::Completed,
            },
            now,
        );
        assert_eq!(completed.state.activity_state, AiActivityState::Completed);

        let returned_to_parent = adapter.adapt(message(
            BrokerDirection::AppServerToCli,
            r#"{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"thread-a","turn":{"id":"turn-c","status":"inProgress"}}}"#,
        ));
        assert!(matches!(
            returned_to_parent.as_slice(),
            [
                AiClientEvent::SessionForked { thread_id },
                AiClientEvent::TurnStarted {
                    thread_id: turn_thread_id,
                    turn_id,
                }
            ] if thread_id == THREAD_A && turn_thread_id == THREAD_A && turn_id == "turn-c"
        ));
        let returned_to_parent = returned_to_parent
            .into_iter()
            .flat_map(|event| reducer.apply(event, now))
            .collect::<Vec<_>>();
        assert_eq!(returned_to_parent.len(), 2);
        assert_eq!(
            returned_to_parent[0].reason,
            AiClientStateChangeReason::SessionForked
        );
        assert_eq!(
            returned_to_parent[1].state.activity_state,
            AiActivityState::Working
        );
    }
}
