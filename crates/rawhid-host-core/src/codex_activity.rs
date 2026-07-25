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
    packet::{AiActivityState, AiClientType, AiClientVariant},
};

const RECONNECT_GRACE: Duration = Duration::from_secs(3);
const MAX_PENDING_CHANGES: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiClientStateChangeReason {
    SessionStarted,
    SessionReplaced,
    SessionEnded,
    TurnStarted,
    TurnCompleted,
    TurnFailed,
    TurnInterrupted,
    RequestStarted,
    RequestResolved,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct AiClientStateSnapshot {
    pub client_type: AiClientType,
    pub client_variant: AiClientVariant,
    pub session_active: bool,
    pub activity_state: AiActivityState,
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
                revision,
            },
            has_emitted: false,
            tracked_thread_id: None,
            tracked_turn_id: None,
            requests: HashMap::new(),
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
            AiClientEvent::TurnStarted { thread_id, turn_id } => {
                if self.tracked_thread_id.as_deref() != Some(thread_id.as_str()) {
                    return Vec::new();
                }
                self.tracked_turn_id = Some(turn_id);
                self.requests.clear();
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
                        self.confirmed_thread_id = None;
                        self.announced_thread_id = None;
                        return vec![AiClientEvent::SessionEnded];
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
                        vec![AiClientEvent::TurnStarted { thread_id, turn_id }]
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
    fn adapter_ends_the_session_when_thread_started_identity_does_not_match() {
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
        assert!(matches!(events.as_slice(), [AiClientEvent::SessionEnded]));
    }
}
