use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc as std_mpsc, Arc, Mutex, RwLock,
    },
    thread,
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use getrandom::fill as fill_random;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tempfile::TempDir;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    process::{Child, ChildStdin, Command},
    sync::{broadcast, mpsc, oneshot},
    task::{JoinHandle, JoinSet},
    time::{self, Instant},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        handshake::derive_accept_key,
        protocol::{Message, Role},
    },
    WebSocketStream,
};

pub const SUPPORTED_CODEX_VERSION: &str = "codex-cli 0.153.2";
pub const SUPPORTED_SCHEMA_SHA256: &str =
    "B06F77062369D481A59CC70720C12B89CB9DD49C385863923262102D3AD6C978";
const COMPATIBLE_CODEX_RELEASES: &[(&str, &str)] = &[
    (SUPPORTED_CODEX_VERSION, SUPPORTED_SCHEMA_SHA256),
    (
        "codex-cli 0.151.0",
        "31AE67BEB2C94CC9509F6A71968600062DC8C6D7FE45437ED3A9129838F4D2D9",
    ),
    (
        "codex-cli 0.150.1",
        "E9BAD0A20736E7D3ABA18C0F04BEF59856FB212AE21049FE17D786682203CFAE",
    ),
    (
        "codex-cli 0.149.1",
        "4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91",
    ),
    (
        "codex-cli 0.149.0",
        "4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91",
    ),
    (
        "codex-cli 0.147.0",
        "BABFD5C98CD978DD858B4762CDFBC9FBA941E1A0E4053DE0050E4082AE1F075A",
    ),
    (
        "codex-cli 0.146.0",
        "D3992FEC1398AFDBEC658DA2C720C6993FBF3C1CE4900785694D2196679EDDFC",
    ),
];
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_CODEX_CLIENTS: usize = 8;
const MANAGED_LAUNCH_PENDING_TIMEOUT: Duration = Duration::from_secs(30);
const MANAGED_LAUNCH_RECONNECT_GRACE: Duration = Duration::from_secs(3);
const MANAGED_LAUNCH_RESULT_RETENTION: Duration = Duration::from_secs(10);
const APPROVAL_RESPONSE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_RESOLVED_APPROVAL_IDS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexAppServerRuntime {
    Windows,
    Wsl {
        distribution: String,
        executable: String,
    },
}

#[derive(Debug, Clone)]
pub struct CodexBrokerConfig {
    pub codex_executable: Option<PathBuf>,
    pub runtime: CodexAppServerRuntime,
    pub version_check_enabled: bool,
    pub app_server_port: u16,
    pub broker_port: u16,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
}

impl Default for CodexBrokerConfig {
    fn default() -> Self {
        Self {
            codex_executable: None,
            runtime: CodexAppServerRuntime::Windows,
            version_check_enabled: false,
            app_server_port: 4500,
            broker_port: 4501,
            startup_timeout: Duration::from_secs(10),
            shutdown_timeout: Duration::from_secs(3),
        }
    }
}

impl CodexBrokerConfig {
    fn validate(&self) -> Result<(), CodexBrokerError> {
        if self.app_server_port < 1024 || self.broker_port < 1024 {
            return Err(CodexBrokerError::InvalidConfig(
                "Codex ports must be in the range 1024..=65535".to_string(),
            ));
        }
        if self.app_server_port == self.broker_port {
            return Err(CodexBrokerError::InvalidConfig(
                "App Server and Broker ports must be different".to_string(),
            ));
        }
        if self.startup_timeout.is_zero() || self.shutdown_timeout.is_zero() {
            return Err(CodexBrokerError::InvalidConfig(
                "startup and shutdown timeouts must be greater than zero".to_string(),
            ));
        }
        if let CodexAppServerRuntime::Wsl {
            distribution,
            executable,
        } = &self.runtime
        {
            if distribution.trim().is_empty() || executable.trim().is_empty() {
                return Err(CodexBrokerError::InvalidConfig(
                    "WSL distribution and Codex executable are required".to_string(),
                ));
            }
            if distribution.contains(['\r', '\n', '\0']) || executable.contains(['\r', '\n', '\0'])
            {
                return Err(CodexBrokerError::InvalidConfig(
                    "WSL distribution and Codex executable cannot contain newline or NUL"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodexBrokerPhase {
    Stopped,
    Starting,
    WaitingForClient,
    Connected,
    Reconnecting,
    Stopping,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct CodexBrokerStatus {
    pub phase: CodexBrokerPhase,
    pub app_server_port: Option<u16>,
    pub broker_port: Option<u16>,
    pub codex_version: Option<String>,
    pub client_connected: bool,
    pub connected_client_count: usize,
    pub max_client_count: usize,
    pub managed_launches: Vec<ManagedLaunchStatus>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ManagedLaunchState {
    WaitingForConnection,
    Connected,
    TimedOut,
    Ended,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ManagedLaunchStatus {
    pub terminal_target_id: String,
    pub display_name: String,
    pub state: ManagedLaunchState,
}

impl Default for CodexBrokerStatus {
    fn default() -> Self {
        Self {
            phase: CodexBrokerPhase::Stopped,
            app_server_port: None,
            broker_port: None,
            codex_version: None,
            client_connected: false,
            connected_client_count: 0,
            max_client_count: MAX_CODEX_CLIENTS,
            managed_launches: Vec::new(),
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexClientLaunchInfo {
    pub runtime: CodexAppServerRuntime,
    pub windows_executable: Option<PathBuf>,
    pub broker_token_path: PathBuf,
    pub broker_port: u16,
    pub terminal_target_id: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrokerDirection {
    CliToAppServer,
    AppServerToCli,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JsonRpcKind {
    Request,
    Response,
    Notification,
    Batch,
    NonJson,
    NonObjectJson,
    Unknown,
    Binary,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JsonRpcMetadata {
    pub kind: JsonRpcKind,
    pub method: Option<String>,
    pub id: Option<Value>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
    pub item_type: Option<String>,
    pub request_id: Option<Value>,
    pub result_thread_id: Option<String>,
    pub response_is_error: bool,
    pub turn_status: Option<String>,
    pub turn_has_error: bool,
    pub will_retry: Option<bool>,
    pub batch_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum CodexBrokerEvent {
    Ready {
        app_server_port: u16,
        broker_port: u16,
    },
    ClientConnected {
        connection_id: String,
    },
    ManagedClientConnected {
        connection_id: String,
        terminal_target_id: String,
    },
    ClientDisconnected {
        connection_id: String,
        origin: &'static str,
    },
    DownstreamAuthRejected,
    AdditionalClientRejected,
    UpstreamConnectFailed,
    Message {
        connection_id: String,
        direction: BrokerDirection,
        metadata: Box<JsonRpcMetadata>,
    },
    /// The full body of one `item/commandExecution/requestApproval`
    /// request, carried on a path separate from `Message` so that
    /// `Message`'s metadata-only shape -- and every existing consumer of
    /// it -- stays untouched. Emitted only for that one method (see
    /// `extract_command_approval_body`), never for the high-frequency
    /// frames Codex CLI otherwise emits (KO-2 observed 640k+ frames in one
    /// run; only requestApproval frames are worth this cost).
    ApprovalRequestBody {
        connection_id: String,
        request_id: Value,
        body: Box<CodexApprovalRequestBody>,
    },
    Error {
        component: &'static str,
        detail: String,
    },
    Stopped,
}

#[derive(Debug, Error)]
pub enum CodexBrokerError {
    #[error("invalid Codex Broker configuration: {0}")]
    InvalidConfig(String),
    #[error("Codex executable was not found: {0}")]
    ExecutableNotFound(String),
    #[error("Codex preflight failed: {0}")]
    Preflight(String),
    #[error("Codex App Server failed: {0}")]
    AppServer(String),
    #[error("Codex Broker failed: {0}")]
    Broker(String),
    #[error("Codex Broker manager is unavailable")]
    ManagerUnavailable,
}

/// Result of attempting to answer a Codex approval through the Broker.
/// `Accepted` is the only case in which a response was written upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexApprovalResponseOutcome {
    Accepted,
    AlreadyResolved,
    RequestNotFound,
    ConnectionNotFound,
}

struct ApprovalResponseCommand {
    request_id: Value,
    decision: Value,
    reply: std_mpsc::Sender<CodexApprovalResponseOutcome>,
    state: Arc<ApprovalResponseState>,
}

#[derive(Default)]
struct ApprovalResponseState(AtomicUsize);

impl ApprovalResponseState {
    const QUEUED: usize = 0;
    const EXECUTING: usize = 1;
    const CANCELLED: usize = 2;

    fn try_start(&self) -> bool {
        self.transition_from_queued(Self::EXECUTING)
    }

    fn try_cancel(&self) -> bool {
        self.transition_from_queued(Self::CANCELLED)
    }

    fn transition_from_queued(&self, next: usize) -> bool {
        self.0
            .compare_exchange(Self::QUEUED, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

fn wait_for_approval_response(
    reply_rx: std_mpsc::Receiver<CodexApprovalResponseOutcome>,
    state: &ApprovalResponseState,
    timeout: Duration,
) -> Result<CodexApprovalResponseOutcome, CodexBrokerError> {
    match reply_rx.recv_timeout(timeout) {
        Ok(outcome) => Ok(outcome),
        Err(std_mpsc::RecvTimeoutError::Timeout) if !state.try_cancel() => {
            // Execution already won the race. Wait for its result instead of
            // reporting a timeout while the response may still be sent upstream.
            reply_rx.recv().map_err(|error| {
                CodexBrokerError::Broker(format!("approval response was not completed: {error}"))
            })
        }
        Err(error) => {
            // Also invalidate queued work if the reply channel disconnected.
            state.try_cancel();
            Err(CodexBrokerError::Broker(format!(
                "approval response was not completed: {error}"
            )))
        }
    }
}

#[derive(Default)]
struct ApprovalArbiter {
    pending: HashSet<String>,
    resolved: HashSet<String>,
    resolved_order: VecDeque<String>,
}

impl ApprovalArbiter {
    fn observe_request(&mut self, request_id: &Value) {
        let token = approval_id_token(request_id);
        self.resolved.remove(&token);
        self.resolved_order.retain(|existing| existing != &token);
        self.pending.insert(token);
    }

    fn observe_resolved(&mut self, request_id: &Value) {
        let token = approval_id_token(request_id);
        self.pending.remove(&token);
        self.mark_resolved(token);
    }

    fn claim(&mut self, request_id: &Value) -> CodexApprovalResponseOutcome {
        let token = approval_id_token(request_id);
        if self.pending.remove(&token) {
            self.mark_resolved(token);
            CodexApprovalResponseOutcome::Accepted
        } else if self.resolved.contains(&token) {
            CodexApprovalResponseOutcome::AlreadyResolved
        } else {
            CodexApprovalResponseOutcome::RequestNotFound
        }
    }

    fn mark_resolved(&mut self, token: String) {
        if self.resolved.insert(token.clone()) {
            self.resolved_order.push_back(token);
        }
        while self.resolved_order.len() > MAX_RESOLVED_APPROVAL_IDS {
            if let Some(oldest) = self.resolved_order.pop_front() {
                self.resolved.remove(&oldest);
            }
        }
    }
}

type ApprovalResponseRoutes =
    Arc<Mutex<HashMap<String, mpsc::UnboundedSender<ApprovalResponseCommand>>>>;

enum ManagerCommand {
    Start(
        CodexBrokerConfig,
        std_mpsc::Sender<Result<CodexBrokerStatus, CodexBrokerError>>,
    ),
    Stop(std_mpsc::Sender<Result<CodexBrokerStatus, CodexBrokerError>>),
    ClientLaunchInfo {
        terminal_target_id: String,
        display_name: String,
        reply: std_mpsc::Sender<Result<CodexClientLaunchInfo, CodexBrokerError>>,
    },
    CancelManagedLaunch(String),
    Shutdown,
}

struct ManagerInner {
    command_tx: mpsc::UnboundedSender<ManagerCommand>,
    event_rx: Mutex<std_mpsc::Receiver<CodexBrokerEvent>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    status: Arc<RwLock<CodexBrokerStatus>>,
    approval_routes: ApprovalResponseRoutes,
}

#[derive(Clone)]
pub struct CodexBrokerManager {
    inner: Arc<ManagerInner>,
}

impl CodexBrokerManager {
    pub fn new() -> Self {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = std_mpsc::channel();
        let status = Arc::new(RwLock::new(CodexBrokerStatus::default()));
        let worker_status = status.clone();
        let approval_routes = Arc::new(Mutex::new(HashMap::new()));
        let worker_approval_routes = approval_routes.clone();
        let worker = thread::Builder::new()
            .name("codex-broker-manager".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .thread_name("codex-broker-runtime")
                    .build();
                match runtime {
                    Ok(runtime) => runtime.block_on(manager_loop(
                        command_rx,
                        event_tx,
                        worker_status.clone(),
                        worker_approval_routes,
                    )),
                    Err(error) => set_error_status(
                        &worker_status,
                        format!("failed to create Broker runtime: {error}"),
                    ),
                }
            })
            .expect("failed to create Codex Broker manager thread");
        let inner = Arc::new(ManagerInner {
            command_tx,
            event_rx: Mutex::new(event_rx),
            worker: Mutex::new(Some(worker)),
            status,
            approval_routes,
        });
        Self { inner }
    }

    pub fn start(&self, config: CodexBrokerConfig) -> Result<CodexBrokerStatus, CodexBrokerError> {
        let (reply_tx, reply_rx) = std_mpsc::channel();
        self.inner
            .command_tx
            .send(ManagerCommand::Start(config, reply_tx))
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?
    }

    pub fn stop(&self) -> Result<CodexBrokerStatus, CodexBrokerError> {
        let (reply_tx, reply_rx) = std_mpsc::channel();
        self.inner
            .command_tx
            .send(ManagerCommand::Stop(reply_tx))
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?
    }

    pub fn status(&self) -> CodexBrokerStatus {
        self.inner.status.read().unwrap().clone()
    }

    pub fn client_launch_info(
        &self,
        terminal_target_id: String,
        display_name: String,
    ) -> Result<CodexClientLaunchInfo, CodexBrokerError> {
        let (reply_tx, reply_rx) = std_mpsc::channel();
        self.inner
            .command_tx
            .send(ManagerCommand::ClientLaunchInfo {
                terminal_target_id,
                display_name,
                reply: reply_tx,
            })
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?;
        reply_rx
            .recv()
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?
    }

    pub fn cancel_managed_launch(&self, terminal_target_id: String) {
        let _ = self
            .inner
            .command_tx
            .send(ManagerCommand::CancelManagedLaunch(terminal_target_id));
    }

    pub fn try_recv_event(&self) -> Result<CodexBrokerEvent, std_mpsc::TryRecvError> {
        self.inner.event_rx.lock().unwrap().try_recv()
    }

    pub fn recv_event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<CodexBrokerEvent, std_mpsc::RecvTimeoutError> {
        self.inner.event_rx.lock().unwrap().recv_timeout(timeout)
    }

    /// Sends one HUD-selected decision to the App Server connection that
    /// owns `request_id`. The per-connection arbiter serializes this with
    /// the CLI's own response, so exactly one side can return `Accepted`.
    pub fn respond_to_approval(
        &self,
        connection_id: &str,
        request_id: Value,
        decision: Value,
    ) -> Result<CodexApprovalResponseOutcome, CodexBrokerError> {
        let route = self
            .inner
            .approval_routes
            .lock()
            .unwrap()
            .get(connection_id)
            .cloned();
        let Some(route) = route else {
            return Ok(CodexApprovalResponseOutcome::ConnectionNotFound);
        };
        let (reply_tx, reply_rx) = std_mpsc::channel();
        let state = Arc::new(ApprovalResponseState::default());
        route
            .send(ApprovalResponseCommand {
                request_id,
                decision,
                reply: reply_tx,
                state: state.clone(),
            })
            .map_err(|_| CodexBrokerError::ManagerUnavailable)?;
        wait_for_approval_response(reply_rx, &state, APPROVAL_RESPONSE_TIMEOUT)
    }
}

impl Default for CodexBrokerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ManagerInner {
    fn drop(&mut self) {
        let _ = self.command_tx.send(ManagerCommand::Shutdown);
        if let Some(worker) = self.worker.lock().unwrap().take() {
            let _ = worker.join();
        }
    }
}

struct Session {
    child: Child,
    child_stdin: Option<ChildStdin>,
    process_tree: ProcessTreeGuard,
    broker_shutdown: Option<oneshot::Sender<()>>,
    broker_task: Option<JoinHandle<Result<(), CodexBrokerError>>>,
    _secrets: TempDir,
    codex_executable: PathBuf,
    managed_launches: Arc<Mutex<ManagedLaunchRegistry>>,
    config: CodexBrokerConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedCredentialState {
    Pending,
    Connected,
    Reconnecting,
    TimedOut,
    Ended,
}

struct ManagedCredential {
    token: String,
    token_path: PathBuf,
    terminal_target_id: String,
    display_name: String,
    state: ManagedCredentialState,
    deadline: Option<Instant>,
    remove_at: Option<Instant>,
}

#[derive(Default)]
struct ManagedLaunchRegistry {
    entries: Vec<ManagedCredential>,
}

impl ManagedLaunchRegistry {
    fn can_issue(&self, terminal_target_id: &str) -> bool {
        self.active_count() < MAX_CODEX_CLIENTS
            && !self
                .entries
                .iter()
                .any(|entry| entry.terminal_target_id == terminal_target_id)
    }

    fn active_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| {
                matches!(
                    entry.state,
                    ManagedCredentialState::Pending
                        | ManagedCredentialState::Connected
                        | ManagedCredentialState::Reconnecting
                )
            })
            .count()
    }

    fn statuses(&self) -> Vec<ManagedLaunchStatus> {
        self.entries
            .iter()
            .map(|entry| ManagedLaunchStatus {
                terminal_target_id: entry.terminal_target_id.clone(),
                display_name: entry.display_name.clone(),
                state: match entry.state {
                    ManagedCredentialState::Pending | ManagedCredentialState::Reconnecting => {
                        ManagedLaunchState::WaitingForConnection
                    }
                    ManagedCredentialState::Connected => ManagedLaunchState::Connected,
                    ManagedCredentialState::TimedOut => ManagedLaunchState::TimedOut,
                    ManagedCredentialState::Ended => ManagedLaunchState::Ended,
                },
            })
            .collect()
    }

    fn authorize(&mut self, token: &str, now: Instant) -> Option<String> {
        let entry = self.entries.iter_mut().find(|entry| {
            constant_time_eq(&entry.token, token)
                && matches!(
                    entry.state,
                    ManagedCredentialState::Pending | ManagedCredentialState::Reconnecting
                )
                && entry.deadline.is_none_or(|deadline| now < deadline)
        })?;
        entry.state = ManagedCredentialState::Connected;
        entry.deadline = None;
        Some(entry.terminal_target_id.clone())
    }

    fn disconnect(&mut self, terminal_target_id: &str, reconnect: bool, now: Instant) {
        let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.terminal_target_id == terminal_target_id)
        else {
            return;
        };
        if reconnect {
            entry.state = ManagedCredentialState::Reconnecting;
            entry.deadline = Some(now + MANAGED_LAUNCH_RECONNECT_GRACE);
        } else {
            entry.state = ManagedCredentialState::Ended;
            entry.deadline = None;
            entry.remove_at = Some(now + MANAGED_LAUNCH_RESULT_RETENTION);
            let _ = fs::remove_file(&entry.token_path);
        }
    }

    fn cancel(&mut self, terminal_target_id: &str) {
        self.entries.retain(|entry| {
            if entry.terminal_target_id == terminal_target_id {
                let _ = fs::remove_file(&entry.token_path);
                false
            } else {
                true
            }
        });
    }

    fn clear(&mut self) {
        for entry in &self.entries {
            let _ = fs::remove_file(&entry.token_path);
        }
        self.entries.clear();
    }

    fn reap(&mut self, now: Instant) {
        for entry in &mut self.entries {
            if matches!(
                entry.state,
                ManagedCredentialState::Pending | ManagedCredentialState::Reconnecting
            ) && entry.deadline.is_some_and(|deadline| now >= deadline)
            {
                entry.state = if entry.state == ManagedCredentialState::Pending {
                    ManagedCredentialState::TimedOut
                } else {
                    ManagedCredentialState::Ended
                };
                entry.deadline = None;
                entry.remove_at = Some(now + MANAGED_LAUNCH_RESULT_RETENTION);
                let _ = fs::remove_file(&entry.token_path);
            }
        }
        self.entries
            .retain(|entry| entry.remove_at.is_none_or(|deadline| now < deadline));
    }
}

async fn manager_loop(
    mut command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
    approval_routes: ApprovalResponseRoutes,
) {
    let mut session: Option<Session> = None;
    let mut health_tick = time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else { break };
                match command {
                    ManagerCommand::Start(config, reply) => {
                        let result = if session.is_some() {
                            Err(CodexBrokerError::InvalidConfig("Codex integration is already running".to_string()))
                        } else {
                            set_starting_status(&status, &config);
                            match start_session(
                                config,
                                event_tx.clone(),
                                status.clone(),
                                approval_routes.clone(),
                            ).await {
                                Ok(started) => {
                                    session = Some(started);
                                    Ok(status.read().unwrap().clone())
                                }
                                Err(error) => {
                                    set_error_status(&status, error.to_string());
                                    Err(error)
                                }
                            }
                        };
                        let _ = reply.send(result);
                    }
                    ManagerCommand::Stop(reply) => {
                        let result = if let Some(current) = session.take() {
                            set_phase(&status, CodexBrokerPhase::Stopping, 0, None);
                            stop_session(current).await;
                            set_stopped_status(&status);
                            let _ = event_tx.send(CodexBrokerEvent::Stopped);
                            Ok(status.read().unwrap().clone())
                        } else {
                            set_stopped_status(&status);
                            Ok(status.read().unwrap().clone())
                        };
                        let _ = reply.send(result);
                    }
                    ManagerCommand::ClientLaunchInfo {
                        terminal_target_id,
                        display_name,
                        reply,
                    } => {
                        let result = session
                            .as_ref()
                            .ok_or_else(|| {
                                CodexBrokerError::InvalidConfig(
                                    "Codex integration is not running".to_string(),
                                )
                            })
                            .and_then(|current| {
                                let phase = status.read().unwrap().phase;
                                let mut launches = current.managed_launches.lock().unwrap();
                                launches.reap(Instant::now());
                                if launches.active_count() >= MAX_CODEX_CLIENTS {
                                    return Err(CodexBrokerError::InvalidConfig(
                                        "Codex CLI connection limit has been reached".to_string(),
                                    ));
                                }
                                if !launches.can_issue(&terminal_target_id) {
                                    return Err(CodexBrokerError::InvalidConfig(
                                        "terminal target ID is already managed".to_string(),
                                    ));
                                }
                                if !matches!(
                                    phase,
                                    CodexBrokerPhase::WaitingForClient
                                        | CodexBrokerPhase::Connected
                                        | CodexBrokerPhase::Reconnecting
                                ) {
                                    return Err(CodexBrokerError::InvalidConfig(format!(
                                        "Codex CLI cannot be launched while integration is {:?}",
                                        phase
                                    )));
                                }
                                let token = generate_token()?;
                                let token_path = current
                                    ._secrets
                                    .path()
                                    .join(format!("client-{token}.token"));
                                write_private_token(&token_path, &token)?;
                                launches.entries.push(ManagedCredential {
                                    token,
                                    token_path: token_path.clone(),
                                    terminal_target_id: terminal_target_id.clone(),
                                    display_name: display_name.clone(),
                                    state: ManagedCredentialState::Pending,
                                    deadline: Some(Instant::now() + MANAGED_LAUNCH_PENDING_TIMEOUT),
                                    remove_at: None,
                                });
                                refresh_managed_launch_status(&status, &launches);
                                Ok(CodexClientLaunchInfo {
                                    runtime: current.config.runtime.clone(),
                                    windows_executable: match current.config.runtime {
                                        CodexAppServerRuntime::Windows => {
                                            Some(current.codex_executable.clone())
                                        }
                                        CodexAppServerRuntime::Wsl { .. } => None,
                                    },
                                    broker_token_path: token_path,
                                    broker_port: current.config.broker_port,
                                    terminal_target_id,
                                    display_name,
                                })
                            });
                        let _ = reply.send(result);
                    }
                    ManagerCommand::CancelManagedLaunch(terminal_target_id) => {
                        if let Some(current) = session.as_ref() {
                            let mut launches = current.managed_launches.lock().unwrap();
                            launches.cancel(&terminal_target_id);
                            refresh_managed_launch_status(&status, &launches);
                        }
                    }
                    ManagerCommand::Shutdown => break,
                }
            }
            _ = health_tick.tick(), if session.is_some() => {
                {
                    let current = session.as_ref().unwrap();
                    let mut launches = current.managed_launches.lock().unwrap();
                    launches.reap(Instant::now());
                    refresh_managed_launch_status(&status, &launches);
                }
                let failure = inspect_session(session.as_mut().unwrap()).await;
                if let Some(detail) = failure {
                    let current = session.take().unwrap();
                    stop_session(current).await;
                    set_error_status(&status, detail.clone());
                    let _ = event_tx.send(CodexBrokerEvent::Error {
                        component: "lifecycle",
                        detail,
                    });
                }
            }
        }
    }
    if let Some(current) = session.take() {
        stop_session(current).await;
    }
    set_stopped_status(&status);
}

async fn start_session(
    config: CodexBrokerConfig,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
    approval_routes: ApprovalResponseRoutes,
) -> Result<Session, CodexBrokerError> {
    config.validate()?;
    ensure_port_available(config.app_server_port, "App Server").await?;
    ensure_port_available(config.broker_port, "Broker").await?;

    let secrets = tempfile::Builder::new()
        .prefix("keylink-codex-")
        .tempdir()
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let executable = match &config.runtime {
        CodexAppServerRuntime::Windows => {
            resolve_codex_executable(config.codex_executable.as_deref()).await?
        }
        CodexAppServerRuntime::Wsl { .. } => PathBuf::from("wsl.exe"),
    };
    let version = match &config.runtime {
        CodexAppServerRuntime::Windows => {
            verify_codex_and_schema(
                &executable,
                secrets.path(),
                config.startup_timeout,
                config.version_check_enabled,
            )
            .await?
        }
        CodexAppServerRuntime::Wsl {
            distribution,
            executable,
        } => {
            verify_wsl_codex_and_schema(
                distribution,
                executable,
                config.startup_timeout,
                config.version_check_enabled,
            )
            .await?
        }
    };

    let app_server_token = generate_token()?;
    // Kept only while constructing the legacy command text below; this value is
    // never accepted by the Broker. Each managed terminal receives its own
    // capability token from `client_launch_info`.
    let app_server_token_path = secrets.path().join("app-server.token");
    write_private_token(&app_server_token_path, &app_server_token)?;
    let _legacy_launcher_message = match &config.runtime {
        CodexAppServerRuntime::Windows => String::new(),
        CodexAppServerRuntime::Wsl { .. } => {
            "設定の「Codexを開く」からWSLのCodex CLIを起動してください".to_string()
        }
    };

    let app_server_url = format!("ws://127.0.0.1:{}", config.app_server_port);
    let mut command = Command::new(&executable);
    match &config.runtime {
        CodexAppServerRuntime::Windows => {
            command
                .arg("app-server")
                .arg("--listen")
                .arg(&app_server_url)
                .arg("--ws-auth")
                .arg("capability-token")
                .arg("--ws-token-file")
                .arg(&app_server_token_path);
        }
        CodexAppServerRuntime::Wsl {
            distribution,
            executable,
        } => {
            let token_path = wsl_path(distribution, &app_server_token_path).await?;
            command
                .arg("--distribution")
                .arg(distribution)
                .arg("--exec")
                .arg("sh")
                .arg("-lc")
                .arg("exec \"$1\" app-server --listen \"$2\" --ws-auth capability-token --ws-token-file \"$3\"")
                .arg("keylink-codex")
                .arg(executable)
                .arg(&app_server_url)
                .arg(token_path);
        }
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_child_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| CodexBrokerError::AppServer(error.to_string()))?;
    let child_stdin = child.stdin.take();
    let process_tree = match ProcessTreeGuard::assign(&child) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            terminate_child(&mut child, child_stdin, config.shutdown_timeout).await;
            return Err(error);
        }
    };
    if let Err(error) =
        wait_for_app_server(&mut child, config.app_server_port, config.startup_timeout).await
    {
        terminate_child(&mut child, child_stdin, config.shutdown_timeout).await;
        return Err(error);
    }

    let listener = match TcpListener::bind(SocketAddr::new(LOOPBACK, config.broker_port)).await {
        Ok(listener) => listener,
        Err(error) => {
            terminate_child(&mut child, child_stdin, config.shutdown_timeout).await;
            return Err(CodexBrokerError::Broker(format!(
                "failed to listen on 127.0.0.1:{}: {error}",
                config.broker_port
            )));
        }
    };
    let (broker_shutdown, shutdown_rx) = oneshot::channel();
    let broker_status = status.clone();
    let broker_events = event_tx.clone();
    let broker_config = config.clone();
    let managed_launches = Arc::new(Mutex::new(ManagedLaunchRegistry::default()));
    let broker_launches = managed_launches.clone();
    let broker_task = tokio::spawn(async move {
        run_broker(
            listener,
            shutdown_rx,
            BrokerRuntimeArgs {
                upstream_url: app_server_url,
                app_server_token,
                upstream_timeout: broker_config.startup_timeout,
                event_tx: broker_events,
                status: broker_status,
                managed_launches: broker_launches,
                approval_routes,
            },
        )
        .await
    });

    {
        let mut current = status.write().unwrap();
        current.phase = CodexBrokerPhase::WaitingForClient;
        current.codex_version = Some(version);
        current.client_connected = false;
        current.connected_client_count = 0;
        current.managed_launches.clear();
        current.last_error = None;
    }
    let _ = event_tx.send(CodexBrokerEvent::Ready {
        app_server_port: config.app_server_port,
        broker_port: config.broker_port,
    });
    Ok(Session {
        child,
        child_stdin,
        process_tree,
        broker_shutdown: Some(broker_shutdown),
        broker_task: Some(broker_task),
        _secrets: secrets,
        codex_executable: executable,
        managed_launches,
        config,
    })
}

async fn inspect_session(session: &mut Session) -> Option<String> {
    match session.child.try_wait() {
        Ok(Some(exit)) => return Some(format!("Codex App Server exited unexpectedly: {exit}")),
        Err(error) => return Some(format!("failed to inspect Codex App Server: {error}")),
        Ok(None) => {}
    }
    if session
        .broker_task
        .as_ref()
        .is_some_and(JoinHandle::is_finished)
    {
        let task = session.broker_task.take().unwrap();
        return Some(match task.await {
            Ok(Ok(())) => "Codex Broker exited unexpectedly".to_string(),
            Ok(Err(error)) => error.to_string(),
            Err(error) => format!("Codex Broker task failed: {error}"),
        });
    }
    None
}

async fn stop_session(mut session: Session) {
    session.managed_launches.lock().unwrap().clear();
    if let Some(shutdown) = session.broker_shutdown.take() {
        let _ = shutdown.send(());
    }
    if let Some(mut task) = session.broker_task.take() {
        if time::timeout(session.config.shutdown_timeout, &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
    }
    terminate_child(
        &mut session.child,
        session.child_stdin.take(),
        session.config.shutdown_timeout,
    )
    .await;
    session.process_tree.terminate();
}

async fn terminate_child(child: &mut Child, stdin: Option<ChildStdin>, timeout: Duration) {
    drop(stdin);
    if time::timeout(timeout, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

/// Owns every process spawned by the App Server launcher on Windows.
///
/// The npm `codex.cmd` shim launches `node.exe`, which in turn launches the
/// native `codex.exe`. Waiting for or killing only the direct child can leave
/// those descendants listening on the configured App Server port. A Job Object
/// makes all descendants owned by this Keylink Studio session and terminates
/// them together on stop, error, or process drop.
#[cfg(windows)]
struct ProcessTreeGuard {
    job: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ProcessTreeGuard {
    fn assign(child: &Child) -> Result<Self, CodexBrokerError> {
        use std::mem::size_of;
        use windows::{
            core::PCWSTR,
            Win32::{
                Foundation::{CloseHandle, HANDLE},
                System::JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
            },
        };

        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.map_err(|error| {
            CodexBrokerError::AppServer(format!("failed to create process job: {error}"))
        })?;
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if let Err(error) = configured {
            unsafe {
                let _ = CloseHandle(job);
            }
            return Err(CodexBrokerError::AppServer(format!(
                "failed to configure process job: {error}"
            )));
        }

        let raw_handle = child.raw_handle().ok_or_else(|| {
            CodexBrokerError::AppServer(
                "App Server process exited before job assignment".to_string(),
            )
        })?;
        let assigned = unsafe { AssignProcessToJobObject(job, HANDLE(raw_handle)) };
        if let Err(error) = assigned {
            unsafe {
                let _ = CloseHandle(job);
            }
            return Err(CodexBrokerError::AppServer(format!(
                "failed to assign App Server process tree: {error}"
            )));
        }
        Ok(Self { job })
    }

    fn terminate(&mut self) {
        use windows::Win32::System::JobObjects::TerminateJobObject;

        if let Err(error) = unsafe { TerminateJobObject(self.job, 1) } {
            tracing::warn!("failed to terminate Codex App Server process tree: {error}");
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTreeGuard {
    fn drop(&mut self) {
        use windows::Win32::Foundation::CloseHandle;

        unsafe {
            let _ = CloseHandle(self.job);
        }
    }
}

#[cfg(not(windows))]
struct ProcessTreeGuard;

#[cfg(not(windows))]
impl ProcessTreeGuard {
    fn assign(_child: &Child) -> Result<Self, CodexBrokerError> {
        Ok(Self)
    }

    fn terminate(&mut self) {}
}

async fn run_broker(
    listener: TcpListener,
    mut shutdown_rx: oneshot::Receiver<()>,
    args: BrokerRuntimeArgs,
) -> Result<(), CodexBrokerError> {
    let reserved_slots = Arc::new(AtomicUsize::new(0));
    let connected_count = Arc::new(AtomicUsize::new(0));
    let reconnect_generation = Arc::new(AtomicU64::new(0));
    let (connection_shutdown, _) = broadcast::channel::<()>(1);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut shutdown_rx => break,
            accepted = listener.accept() => {
                let (socket, peer) = accepted.map_err(|error| CodexBrokerError::Broker(error.to_string()))?;
                if peer.ip() != LOOPBACK {
                    drop(socket);
                    continue;
                }
                let args = ConnectionArgs {
                    upstream_url: args.upstream_url.clone(),
                    app_server_token: args.app_server_token.clone(),
                    upstream_timeout: args.upstream_timeout,
                    reserved_slots: reserved_slots.clone(),
                    connected_count: connected_count.clone(),
                    reconnect_generation: reconnect_generation.clone(),
                    event_tx: args.event_tx.clone(),
                    status: args.status.clone(),
                    managed_launches: args.managed_launches.clone(),
                    approval_routes: args.approval_routes.clone(),
                    shutdown_rx: connection_shutdown.subscribe(),
                };
                connections.spawn(async move { handle_connection(socket, args).await });
            }
            finished = connections.join_next(), if !connections.is_empty() => {
                match finished {
                    Some(Ok(Err(error))) => {
                        let _ = args.event_tx.send(CodexBrokerEvent::Error {
                            component: "connection",
                            detail: error.to_string(),
                        });
                    }
                    Some(Err(error)) => {
                        return Err(CodexBrokerError::Broker(format!("connection task failed: {error}")));
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = connection_shutdown.send(());
    while let Some(result) = connections.join_next().await {
        if let Err(error) = result {
            tracing::warn!("Codex Broker connection cleanup failed: {error}");
        }
    }
    Ok(())
}

struct BrokerRuntimeArgs {
    upstream_url: String,
    app_server_token: String,
    upstream_timeout: Duration,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
    managed_launches: Arc<Mutex<ManagedLaunchRegistry>>,
    approval_routes: ApprovalResponseRoutes,
}

struct ConnectionArgs {
    upstream_url: String,
    app_server_token: String,
    upstream_timeout: Duration,
    reserved_slots: Arc<AtomicUsize>,
    connected_count: Arc<AtomicUsize>,
    reconnect_generation: Arc<AtomicU64>,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
    managed_launches: Arc<Mutex<ManagedLaunchRegistry>>,
    approval_routes: ApprovalResponseRoutes,
    shutdown_rx: broadcast::Receiver<()>,
}

struct ConnectionSlotGuard {
    reserved_slots: Arc<AtomicUsize>,
    connected_count: Arc<AtomicUsize>,
    connected: bool,
}

impl ConnectionSlotGuard {
    fn promote(&mut self) {
        if !self.connected {
            self.connected_count.fetch_add(1, Ordering::AcqRel);
            self.connected = true;
        }
    }
}

impl Drop for ConnectionSlotGuard {
    fn drop(&mut self) {
        if self.connected {
            self.connected_count.fetch_sub(1, Ordering::AcqRel);
        }
        self.reserved_slots.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_acquire_client_slot(connected_count: &AtomicUsize) -> bool {
    connected_count
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_CODEX_CLIENTS).then_some(count + 1)
        })
        .is_ok()
}

async fn handle_connection(
    mut downstream: TcpStream,
    mut args: ConnectionArgs,
) -> Result<(), CodexBrokerError> {
    let request = match read_upgrade_request(&mut downstream, &mut args.shutdown_rx).await {
        Ok(request) => request,
        Err(HandshakeFailure::Shutdown) => return Ok(()),
        Err(HandshakeFailure::Protocol(message)) => {
            reject_upgrade(&mut downstream, "400 Bad Request", &message).await;
            return Ok(());
        }
        Err(HandshakeFailure::Io(error)) => return Err(CodexBrokerError::Broker(error)),
    };
    let terminal_target_id = {
        let mut launches = args.managed_launches.lock().unwrap();
        launches.reap(Instant::now());
        let authorized = launches.authorize(
            request.bearer_token.as_deref().unwrap_or(""),
            Instant::now(),
        );
        refresh_managed_launch_status(&args.status, &launches);
        authorized
    };
    let Some(terminal_target_id) = terminal_target_id else {
        let _ = args.event_tx.send(CodexBrokerEvent::DownstreamAuthRejected);
        reject_upgrade(&mut downstream, "401 Unauthorized", "Unauthorized").await;
        return Ok(());
    };
    if !try_acquire_client_slot(&args.reserved_slots) {
        let _ = args
            .event_tx
            .send(CodexBrokerEvent::AdditionalClientRejected);
        reject_upgrade(
            &mut downstream,
            "409 Conflict",
            "Codex CLI connection limit reached",
        )
        .await;
        return Ok(());
    }
    let mut slot_guard = ConnectionSlotGuard {
        reserved_slots: args.reserved_slots.clone(),
        connected_count: args.connected_count.clone(),
        connected: false,
    };
    let mut upstream_request = args
        .upstream_url
        .as_str()
        .into_client_request()
        .map_err(|error| CodexBrokerError::Broker(error.to_string()))?;
    upstream_request.headers_mut().insert(
        "authorization",
        format!("Bearer {}", args.app_server_token)
            .parse()
            .map_err(|error| {
                CodexBrokerError::Broker(format!("invalid authorization header: {error}"))
            })?,
    );
    let upstream = tokio::select! {
        _ = args.shutdown_rx.recv() => return Ok(()),
        result = time::timeout(args.upstream_timeout, connect_async(upstream_request)) => {
            match result {
                Ok(Ok((socket, _response))) => socket,
                _ => {
                    let _ = args.event_tx.send(CodexBrokerEvent::UpstreamConnectFailed);
                    {
                        let mut launches = args.managed_launches.lock().unwrap();
                        launches.disconnect(&terminal_target_id, false, Instant::now());
                        refresh_managed_launch_status(&args.status, &launches);
                    }
                    reject_upgrade(&mut downstream, "502 Bad Gateway", "App Server connection failed").await;
                    return Ok(());
                }
            }
        }
    };

    accept_upgrade(&mut downstream, &request.websocket_key).await?;
    let downstream = WebSocketStream::from_raw_socket(downstream, Role::Server, None).await;
    let connection_id = random_identifier()?;
    let (approval_tx, approval_rx) = mpsc::unbounded_channel();
    args.approval_routes
        .lock()
        .unwrap()
        .insert(connection_id.clone(), approval_tx);
    slot_guard.promote();
    args.reconnect_generation.fetch_add(1, Ordering::AcqRel);
    sync_connection_status(
        &args.status,
        &args.connected_count,
        CodexBrokerPhase::WaitingForClient,
    );
    let _ = args
        .event_tx
        .send(CodexBrokerEvent::ManagedClientConnected {
            connection_id: connection_id.clone(),
            terminal_target_id: terminal_target_id.clone(),
        });
    let origin = forward_messages(
        downstream,
        upstream,
        &connection_id,
        &args.event_tx,
        &mut args.shutdown_rx,
        approval_rx,
    )
    .await;
    args.approval_routes.lock().unwrap().remove(&connection_id);
    drop(slot_guard);
    let remaining = sync_connection_status(
        &args.status,
        &args.connected_count,
        if origin == "cli" {
            CodexBrokerPhase::Reconnecting
        } else {
            CodexBrokerPhase::WaitingForClient
        },
    );
    let reconnecting = origin == "cli" && remaining == 0;
    {
        let mut launches = args.managed_launches.lock().unwrap();
        launches.disconnect(&terminal_target_id, origin == "cli", Instant::now());
        refresh_managed_launch_status(&args.status, &launches);
    }
    if reconnecting {
        let generation = args
            .reconnect_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        schedule_waiting_for_client_after_grace(
            args.status.clone(),
            args.reconnect_generation.clone(),
            generation,
        );
    }
    let _ = args.event_tx.send(CodexBrokerEvent::ClientDisconnected {
        connection_id,
        origin,
    });
    Ok(())
}

async fn forward_messages<Upstream>(
    downstream: WebSocketStream<TcpStream>,
    upstream: WebSocketStream<Upstream>,
    connection_id: &str,
    event_tx: &std_mpsc::Sender<CodexBrokerEvent>,
    shutdown_rx: &mut broadcast::Receiver<()>,
    mut approval_rx: mpsc::UnboundedReceiver<ApprovalResponseCommand>,
) -> &'static str
where
    Upstream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut downstream_write, mut downstream_read) = downstream.split();
    let (mut upstream_write, mut upstream_read) = upstream.split();
    let mut approval_arbiter = ApprovalArbiter::default();
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                let _ = downstream_write.send(Message::Close(None)).await;
                let _ = upstream_write.send(Message::Close(None)).await;
                return "broker";
            }
            message = downstream_read.next() => {
                let Some(Ok(message)) = message else {
                    let _ = upstream_write.send(Message::Close(None)).await;
                    return "cli";
                };
                let metadata = message_metadata(&message);
                emit_message_metadata(event_tx, connection_id, BrokerDirection::CliToAppServer, &message);
                if let Some(metadata) = metadata.as_ref() {
                    if metadata.kind == JsonRpcKind::Response {
                        if let Some(id) = metadata.id.as_ref() {
                            let outcome = approval_arbiter.claim(id);
                            if outcome == CodexApprovalResponseOutcome::AlreadyResolved {
                                tracing::debug!(
                                    %connection_id,
                                    request_id = %approval_id_token(id),
                                    "ignored late Codex CLI approval response"
                                );
                                continue;
                            }
                        }
                    }
                }
                let closed = message.is_close();
                if upstream_write.send(message).await.is_err() || closed {
                    return "cli";
                }
            }
            message = upstream_read.next() => {
                let Some(Ok(message)) = message else {
                    let _ = downstream_write.send(Message::Close(None)).await;
                    return "app_server";
                };
                let metadata = message_metadata(&message);
                emit_message_metadata(event_tx, connection_id, BrokerDirection::AppServerToCli, &message);
                if let Some(metadata) = metadata.as_ref() {
                    if metadata.kind == JsonRpcKind::Request
                        && metadata.method.as_deref()
                            == Some("item/commandExecution/requestApproval")
                    {
                        if let Some(id) = metadata.id.as_ref() {
                            approval_arbiter.observe_request(id);
                        }
                    } else if metadata.method.as_deref() == Some("serverRequest/resolved") {
                        if let Some(id) = metadata.request_id.as_ref() {
                            approval_arbiter.observe_resolved(id);
                        }
                    }
                }
                let closed = message.is_close();
                if downstream_write.send(message).await.is_err() || closed {
                    return "app_server";
                }
            }
            command = approval_rx.recv() => {
                let Some(command) = command else { continue };
                if !command.state.try_start() {
                    continue;
                }
                let outcome = approval_arbiter.claim(&command.request_id);
                let outcome = if outcome == CodexApprovalResponseOutcome::Accepted {
                    let response = Message::Text(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": command.request_id,
                        "result": { "decision": command.decision },
                    }).to_string().into());
                    emit_message_metadata(
                        event_tx,
                        connection_id,
                        BrokerDirection::CliToAppServer,
                        &response,
                    );
                    if upstream_write.send(response).await.is_err() {
                        let _ = command.reply.send(CodexApprovalResponseOutcome::RequestNotFound);
                        return "app_server";
                    }
                    CodexApprovalResponseOutcome::Accepted
                } else {
                    tracing::debug!(
                        %connection_id,
                        request_id = %approval_id_token(&command.request_id),
                        ?outcome,
                        "ignored Codex HUD approval response"
                    );
                    outcome
                };
                let _ = command.reply.send(outcome);
            }
        }
    }
}

fn message_metadata(message: &Message) -> Option<JsonRpcMetadata> {
    match message {
        Message::Text(text) => Some(classify_json_rpc(text.as_str())),
        Message::Binary(_) => Some(empty_metadata(JsonRpcKind::Binary)),
        _ => None,
    }
}

fn approval_id_token(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::String(value) => format!("s:{value}"),
        Value::Number(value) => format!("n:{value}"),
        other => format!("?:{other}"),
    }
}

fn emit_message_metadata(
    event_tx: &std_mpsc::Sender<CodexBrokerEvent>,
    connection_id: &str,
    direction: BrokerDirection,
    message: &Message,
) {
    let metadata = match message {
        Message::Text(text) => classify_json_rpc(text.as_str()),
        Message::Binary(_) => JsonRpcMetadata {
            kind: JsonRpcKind::Binary,
            method: None,
            id: None,
            thread_id: None,
            turn_id: None,
            item_id: None,
            item_type: None,
            request_id: None,
            result_thread_id: None,
            response_is_error: false,
            turn_status: None,
            turn_has_error: false,
            will_retry: None,
            batch_count: None,
        },
        _ => return,
    };
    // Command approval requests are the one frame whose body a future HUD
    // needs (see docs/ai-approval-hud-design.md §7.2). This is a second,
    // independent parse of the same text -- `classify_json_rpc` above is
    // never touched to make this possible, and every other frame (Codex
    // CLI emits these at high frequency) skips this branch entirely.
    if direction == BrokerDirection::AppServerToCli
        && metadata.kind == JsonRpcKind::Request
        && metadata.method.as_deref() == Some("item/commandExecution/requestApproval")
    {
        if let Message::Text(text) = message {
            if let Some(body) = extract_command_approval_body(text.as_str()) {
                let _ = event_tx.send(CodexBrokerEvent::ApprovalRequestBody {
                    connection_id: connection_id.to_string(),
                    request_id: metadata.id.clone().unwrap_or(Value::Null),
                    body: Box::new(body),
                });
            }
        }
    }
    let _ = event_tx.send(CodexBrokerEvent::Message {
        connection_id: connection_id.to_string(),
        direction,
        metadata: Box::new(metadata),
    });
}

/// The body of a Codex `item/commandExecution/requestApproval` request,
/// captured verbatim from the wire (see the real example in
/// `docs/codex-approval-proxy-gate-results.md` §5). Nothing here is
/// reconstructed or summarized: `available_decisions` in particular is
/// kept exactly as received, opaque-element by opaque-element (§5.1 of
/// that document -- elements mix plain strings and objects, and the set
/// changes per request).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CodexApprovalRequestBody {
    /// `params.commandActions[].command`, in order. This is the primary
    /// display text -- `params.command` is the full wrapped invocation
    /// (e.g. a `powershell.exe -Command '...'` shell wrapper) and buries
    /// the point.
    pub command_actions: Vec<String>,
    pub command: Option<String>,
    pub reason: Option<String>,
    pub cwd: Option<String>,
    pub kind: Option<String>,
    pub available_decisions: Vec<Value>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
}

/// Extracts the full body of a `item/commandExecution/requestApproval`
/// request from its raw JSON-RPC text. Returns `None` for anything that is
/// not a well-formed request of that shape (e.g. missing `params`). Callers
/// are expected to check the method first (see `emit_message_metadata`);
/// this function does not check it itself.
pub fn extract_command_approval_body(text: &str) -> Option<CodexApprovalRequestBody> {
    let value: Value = serde_json::from_str(text).ok()?;
    let params = value.get("params")?.as_object()?;
    let command_actions = params
        .get("commandActions")
        .and_then(Value::as_array)
        .map(|actions| {
            actions
                .iter()
                .filter_map(|action| action.get("command").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let available_decisions = params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Some(CodexApprovalRequestBody {
        command_actions,
        command: params
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_string),
        reason: params
            .get("reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        cwd: params
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string),
        kind: params
            .get("kind")
            .and_then(Value::as_str)
            .map(str::to_string),
        available_decisions,
        thread_id: params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string),
        turn_id: params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_string),
        item_id: params
            .get("itemId")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

struct UpgradeRequest {
    websocket_key: String,
    bearer_token: Option<String>,
}

enum HandshakeFailure {
    Shutdown,
    Protocol(String),
    Io(String),
}

async fn read_upgrade_request(
    stream: &mut TcpStream,
    shutdown_rx: &mut broadcast::Receiver<()>,
) -> Result<UpgradeRequest, HandshakeFailure> {
    let mut buffer = Vec::with_capacity(1024);
    loop {
        if buffer.len() >= MAX_HTTP_HEADER_BYTES {
            return Err(HandshakeFailure::Protocol(
                "HTTP upgrade header is too large".to_string(),
            ));
        }
        let mut chunk = [0_u8; 1024];
        let read = tokio::select! {
            _ = shutdown_rx.recv() => return Err(HandshakeFailure::Shutdown),
            result = stream.read(&mut chunk) => result.map_err(|error| HandshakeFailure::Io(error.to_string()))?,
        };
        if read == 0 {
            return Err(HandshakeFailure::Protocol(
                "connection closed before WebSocket upgrade".to_string(),
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(end) = buffer.windows(4).position(|part| part == b"\r\n\r\n") {
            if end + 4 != buffer.len() {
                return Err(HandshakeFailure::Protocol(
                    "unexpected data before WebSocket upgrade completed".to_string(),
                ));
            }
            break;
        }
    }
    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Request::new(&mut headers);
    if !parsed
        .parse(&buffer)
        .map_err(|error| HandshakeFailure::Protocol(error.to_string()))?
        .is_complete()
    {
        return Err(HandshakeFailure::Protocol(
            "incomplete WebSocket upgrade".to_string(),
        ));
    }
    if parsed.method != Some("GET") {
        return Err(HandshakeFailure::Protocol(
            "WebSocket upgrade must use GET".to_string(),
        ));
    }
    let header = |name: &str| {
        parsed
            .headers
            .iter()
            .find(|header| header.name.eq_ignore_ascii_case(name))
            .and_then(|header| std::str::from_utf8(header.value).ok())
    };
    let upgrade = header("upgrade").unwrap_or("");
    let connection = header("connection").unwrap_or("");
    let version = header("sec-websocket-version").unwrap_or("");
    let websocket_key = header("sec-websocket-key").unwrap_or("");
    if !upgrade.eq_ignore_ascii_case("websocket")
        || !connection
            .split(',')
            .any(|part| part.trim().eq_ignore_ascii_case("upgrade"))
        || version != "13"
        || websocket_key.is_empty()
    {
        return Err(HandshakeFailure::Protocol(
            "invalid WebSocket upgrade headers".to_string(),
        ));
    }
    let bearer_token = header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::to_string);
    Ok(UpgradeRequest {
        websocket_key: websocket_key.to_string(),
        bearer_token,
    })
}

async fn accept_upgrade(stream: &mut TcpStream, key: &str) -> Result<(), CodexBrokerError> {
    let accept = derive_accept_key(key.as_bytes());
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| CodexBrokerError::Broker(error.to_string()))
}

async fn reject_upgrade(stream: &mut TcpStream, status: &str, message: &str) {
    let body = format!("{message}\n");
    let response = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

pub fn classify_json_rpc(text: &str) -> JsonRpcMetadata {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return empty_metadata(JsonRpcKind::NonJson),
    };
    if let Some(batch) = value.as_array() {
        return JsonRpcMetadata {
            kind: JsonRpcKind::Batch,
            method: None,
            id: None,
            thread_id: None,
            turn_id: None,
            item_id: None,
            item_type: None,
            request_id: None,
            result_thread_id: None,
            response_is_error: false,
            turn_status: None,
            turn_has_error: false,
            will_retry: None,
            batch_count: Some(batch.len()),
        };
    }
    let Some(object) = value.as_object() else {
        return empty_metadata(JsonRpcKind::NonObjectJson);
    };
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(str::to_string);
    let id = object
        .get("id")
        .filter(|id| id.is_null() || id.is_string() || id.is_i64() || id.is_u64() || id.is_f64())
        .cloned();
    let kind = match (
        method.is_some(),
        object.contains_key("id"),
        object.contains_key("result") || object.contains_key("error"),
    ) {
        (true, true, _) => JsonRpcKind::Request,
        (true, false, _) => JsonRpcKind::Notification,
        (false, true, true) => JsonRpcKind::Response,
        _ => JsonRpcKind::Unknown,
    };
    JsonRpcMetadata {
        kind,
        method,
        id,
        thread_id: extract_identifier(&value, &["threadId", "thread_id"])
            .or_else(|| string_at(&value, &["params", "thread", "id"])),
        turn_id: extract_identifier(&value, &["turnId", "turn_id"])
            .or_else(|| string_at(&value, &["params", "turn", "id"])),
        item_id: extract_identifier(&value, &["itemId", "item_id"])
            .or_else(|| string_at(&value, &["params", "item", "id"])),
        item_type: string_at(&value, &["params", "item", "type"]),
        request_id: scalar_at(&value, &["params", "requestId"]),
        result_thread_id: string_at(&value, &["result", "thread", "id"]),
        response_is_error: object.contains_key("error"),
        turn_status: string_at(&value, &["params", "turn", "status"]),
        turn_has_error: value_at(&value, &["params", "turn", "error"])
            .is_some_and(|error| !error.is_null()),
        will_retry: value_at(&value, &["params", "willRetry"]).and_then(Value::as_bool),
        batch_count: None,
    }
}

fn empty_metadata(kind: JsonRpcKind) -> JsonRpcMetadata {
    JsonRpcMetadata {
        kind,
        method: None,
        id: None,
        thread_id: None,
        turn_id: None,
        item_id: None,
        item_type: None,
        request_id: None,
        result_thread_id: None,
        response_is_error: false,
        turn_status: None,
        turn_has_error: false,
        will_retry: None,
        batch_count: None,
    }
}

fn extract_identifier(value: &Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(identifier) = object.get(*key).and_then(Value::as_str) {
            return Some(identifier.to_string());
        }
    }
    for container in ["params", "result", "thread", "turn", "item"] {
        if let Some(identifier) = object
            .get(container)
            .and_then(|nested| extract_identifier(nested, keys))
        {
            return Some(identifier);
        }
    }
    None
}

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    value_at(value, path)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn scalar_at(value: &Value, path: &[&str]) -> Option<Value> {
    value_at(value, path)
        .filter(|item| {
            item.is_null() || item.is_string() || item.is_i64() || item.is_u64() || item.is_f64()
        })
        .cloned()
}

async fn ensure_port_available(port: u16, component: &str) -> Result<(), CodexBrokerError> {
    TcpListener::bind(SocketAddr::new(LOOPBACK, port))
        .await
        .map(drop)
        .map_err(|error| {
            CodexBrokerError::InvalidConfig(format!(
                "{component} port 127.0.0.1:{port} is unavailable: {error}"
            ))
        })
}

async fn resolve_codex_executable(configured: Option<&Path>) -> Result<PathBuf, CodexBrokerError> {
    if let Some(path) = configured {
        if !path.is_absolute() || !path.is_file() {
            return Err(CodexBrokerError::ExecutableNotFound(
                path.display().to_string(),
            ));
        }
        return fs::canonicalize(path)
            .map_err(|_| CodexBrokerError::ExecutableNotFound(path.display().to_string()));
    }
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("where.exe", &["codex"])
    } else {
        ("which", &["codex"])
    };
    let output = Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|error| CodexBrokerError::ExecutableNotFound(error.to_string()))?;
    if !output.status.success() {
        return Err(CodexBrokerError::ExecutableNotFound(
            "codex was not found on PATH".to_string(),
        ));
    }
    let path = select_codex_executable(&output.stdout).ok_or_else(|| {
        CodexBrokerError::ExecutableNotFound("codex was not found on PATH".to_string())
    })?;
    fs::canonicalize(&path)
        .map_err(|_| CodexBrokerError::ExecutableNotFound(path.display().to_string()))
}

fn select_codex_executable(output: &[u8]) -> Option<PathBuf> {
    String::from_utf8_lossy(output)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|path| {
            if !cfg!(windows) {
                return true;
            }
            path.extension().is_some_and(|extension| {
                matches!(
                    extension.to_string_lossy().to_ascii_lowercase().as_str(),
                    "com" | "exe" | "bat" | "cmd"
                )
            })
        })
}

async fn verify_codex_and_schema(
    executable: &Path,
    work_dir: &Path,
    timeout: Duration,
    version_check_enabled: bool,
) -> Result<String, CodexBrokerError> {
    let version_output = time::timeout(timeout, Command::new(executable).arg("--version").output())
        .await
        .map_err(|_| CodexBrokerError::Preflight("codex --version timed out".to_string()))?
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    if !version_output.status.success() {
        return Err(CodexBrokerError::Preflight(
            "codex --version failed".to_string(),
        ));
    }
    if version_check_enabled && compatible_schema_sha256(&version).is_none() {
        return Err(CodexBrokerError::Preflight(format!(
            "requires one of {}; detected {version:?}",
            compatible_codex_versions()
        )));
    }
    let schema_dir = work_dir.join("schema");
    fs::create_dir(&schema_dir).map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let schema_output = time::timeout(
        timeout,
        Command::new(executable)
            .arg("app-server")
            .arg("generate-json-schema")
            .arg("--experimental")
            .arg("--out")
            .arg(&schema_dir)
            .output(),
    )
    .await
    .map_err(|_| CodexBrokerError::Preflight("Schema generation timed out".to_string()))?
    .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    if !schema_output.status.success() {
        return Err(CodexBrokerError::Preflight(
            "Schema generation failed".to_string(),
        ));
    }
    let schema_path = schema_dir.join("codex_app_server_protocol.schemas.json");
    let schema =
        fs::read(&schema_path).map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let actual_hash = hex::encode_upper(Sha256::digest(&schema));
    if !schema_is_compatible(&version, &actual_hash, version_check_enabled) {
        return Err(CodexBrokerError::Preflight(format!(
            "unsupported Codex App Server Schema SHA-256: {actual_hash}"
        )));
    }
    Ok(version)
}

async fn verify_wsl_codex_and_schema(
    distribution: &str,
    executable: &str,
    timeout: Duration,
    version_check_enabled: bool,
) -> Result<String, CodexBrokerError> {
    let version_output = time::timeout(
        timeout,
        Command::new("wsl.exe")
            .arg("--distribution")
            .arg(distribution)
            .arg("--exec")
            .arg("sh")
            .arg("-lc")
            .arg("exec \"$1\" --version")
            .arg("keylink-codex")
            .arg(executable)
            .output(),
    )
    .await
    .map_err(|_| CodexBrokerError::Preflight("WSL codex --version timed out".to_string()))?
    .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    if !version_output.status.success() {
        return Err(CodexBrokerError::Preflight(
            "WSL codex --version failed".to_string(),
        ));
    }
    if version_check_enabled && compatible_schema_sha256(&version).is_none() {
        return Err(CodexBrokerError::Preflight(format!(
            "requires one of {}; detected {version:?} in WSL",
            compatible_codex_versions()
        )));
    }

    let schema_dir = format!("/tmp/keylink-codex-schema-{}", random_identifier()?);
    let script = "set -eu; dir=$2; trap 'rm -rf \"$dir\"' EXIT; mkdir -p \"$dir\"; \"$1\" app-server generate-json-schema --experimental --out \"$dir\"; sha256sum \"$dir/codex_app_server_protocol.schemas.json\"";
    let schema_output = time::timeout(
        timeout,
        Command::new("wsl.exe")
            .arg("--distribution")
            .arg(distribution)
            .arg("--exec")
            .arg("sh")
            .arg("-lc")
            .arg(script)
            .arg("keylink-codex")
            .arg(executable)
            .arg(&schema_dir)
            .output(),
    )
    .await
    .map_err(|_| CodexBrokerError::Preflight("WSL schema generation timed out".to_string()))?
    .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    if !schema_output.status.success() {
        return Err(CodexBrokerError::Preflight(
            "WSL schema generation failed".to_string(),
        ));
    }
    let actual_hash = String::from_utf8_lossy(&schema_output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if !schema_is_compatible(&version, &actual_hash, version_check_enabled) {
        return Err(CodexBrokerError::Preflight(format!(
            "unsupported WSL Codex App Server Schema SHA-256: {actual_hash}"
        )));
    }
    Ok(version)
}

fn compatible_schema_sha256(version: &str) -> Option<&'static str> {
    COMPATIBLE_CODEX_RELEASES
        .iter()
        .find_map(|(candidate, hash)| (*candidate == version).then_some(*hash))
}

fn schema_is_compatible(version: &str, actual_hash: &str, version_check_enabled: bool) -> bool {
    if version_check_enabled {
        return compatible_schema_sha256(version) == Some(actual_hash);
    }
    COMPATIBLE_CODEX_RELEASES
        .iter()
        .any(|(_, hash)| *hash == actual_hash)
}

fn compatible_codex_versions() -> String {
    COMPATIBLE_CODEX_RELEASES
        .iter()
        .map(|(version, _)| *version)
        .collect::<Vec<_>>()
        .join(", ")
}

async fn wsl_path(distribution: &str, path: &Path) -> Result<String, CodexBrokerError> {
    let path = display_path(path);
    let output = Command::new("wsl.exe")
        .arg("--distribution")
        .arg(distribution)
        .arg("--exec")
        .arg("sh")
        .arg("-lc")
        .arg("wslpath -u \"$1\"")
        .arg("keylink-codex")
        .arg(path)
        .output()
        .await
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let converted = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || !converted.starts_with('/') {
        return Err(CodexBrokerError::Preflight(
            "failed to convert App Server token path for WSL".to_string(),
        ));
    }
    Ok(converted)
}

async fn wait_for_app_server(
    child: &mut Child,
    port: u16,
    timeout: Duration,
) -> Result<(), CodexBrokerError> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(exit) = child
            .try_wait()
            .map_err(|error| CodexBrokerError::AppServer(error.to_string()))?
        {
            return Err(CodexBrokerError::AppServer(format!(
                "process exited before listening: {exit}"
            )));
        }
        if TcpStream::connect(SocketAddr::new(LOOPBACK, port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(CodexBrokerError::AppServer(format!(
                "listen timeout on 127.0.0.1:{port}"
            )));
        }
        time::sleep(Duration::from_millis(100)).await;
    }
}

fn generate_token() -> Result<String, CodexBrokerError> {
    let mut bytes = [0_u8; 32];
    fill_random(&mut bytes).map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    Ok(hex::encode(bytes))
}

fn random_identifier() -> Result<String, CodexBrokerError> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes).map_err(|error| CodexBrokerError::Broker(error.to_string()))?;
    Ok(hex::encode(bytes))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
    }
}

fn write_private_token(path: &Path, token: &str) -> Result<(), CodexBrokerError> {
    fs::write(path, token).map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    set_private_file_permissions(path).inspect_err(|_| {
        let _ = fs::remove_file(path);
    })
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> Result<(), CodexBrokerError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))
}

#[cfg(windows)]
fn set_private_file_permissions(path: &Path) -> Result<(), CodexBrokerError> {
    let sid_output = std::process::Command::new("whoami.exe")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    if !sid_output.status.success() {
        return Err(CodexBrokerError::Preflight(
            "failed to resolve current Windows user SID".to_string(),
        ));
    }
    let row = String::from_utf8_lossy(&sid_output.stdout);
    let sid = row
        .split(',')
        .nth(1)
        .map(|value| value.trim().trim_matches('"'))
        .filter(|value| value.starts_with("S-1-"))
        .ok_or_else(|| {
            CodexBrokerError::Preflight("invalid Windows user SID response".to_string())
        })?;
    let grant = format!("*{sid}:(F)");
    let output = std::process::Command::new("icacls.exe")
        .arg(path)
        .args(["/inheritance:r", "/grant:r"])
        .arg(grant)
        .output()
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    if !output.status.success() {
        return Err(CodexBrokerError::Preflight(
            "failed to restrict token file ACL".to_string(),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn set_private_file_permissions(_path: &Path) -> Result<(), CodexBrokerError> {
    Err(CodexBrokerError::Preflight(
        "private token files are unsupported on this platform".to_string(),
    ))
}

#[cfg(windows)]
fn hide_child_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_child_window(_command: &mut Command) {}

fn set_starting_status(status: &Arc<RwLock<CodexBrokerStatus>>, config: &CodexBrokerConfig) {
    let mut current = status.write().unwrap();
    current.phase = CodexBrokerPhase::Starting;
    current.app_server_port = Some(config.app_server_port);
    current.broker_port = Some(config.broker_port);
    current.codex_version = None;
    current.client_connected = false;
    current.connected_client_count = 0;
    current.managed_launches.clear();
    current.last_error = None;
}

fn set_stopped_status(status: &Arc<RwLock<CodexBrokerStatus>>) {
    *status.write().unwrap() = CodexBrokerStatus::default();
}

fn set_error_status(status: &Arc<RwLock<CodexBrokerStatus>>, detail: String) {
    let mut current = status.write().unwrap();
    current.phase = CodexBrokerPhase::Error;
    current.client_connected = false;
    current.connected_client_count = 0;
    current.managed_launches.clear();
    current.last_error = Some(detail);
}

fn refresh_managed_launch_status(
    status: &Arc<RwLock<CodexBrokerStatus>>,
    launches: &ManagedLaunchRegistry,
) {
    status.write().unwrap().managed_launches = launches.statuses();
}

fn set_phase(
    status: &Arc<RwLock<CodexBrokerStatus>>,
    phase: CodexBrokerPhase,
    connected_client_count: usize,
    error: Option<String>,
) {
    let mut current = status.write().unwrap();
    current.phase = phase;
    current.connected_client_count = connected_client_count;
    current.client_connected = connected_client_count > 0;
    current.last_error = error;
}

fn sync_connection_status(
    status: &Arc<RwLock<CodexBrokerStatus>>,
    connected_count: &AtomicUsize,
    no_client_phase: CodexBrokerPhase,
) -> usize {
    let mut current = status.write().unwrap();
    let count = connected_count.load(Ordering::Acquire);
    if matches!(
        current.phase,
        CodexBrokerPhase::Stopping | CodexBrokerPhase::Error
    ) {
        return count;
    }
    current.phase = if count > 0 {
        CodexBrokerPhase::Connected
    } else {
        no_client_phase
    };
    current.connected_client_count = count;
    current.client_connected = count > 0;
    count
}

fn schedule_waiting_for_client_after_grace(
    status: Arc<RwLock<CodexBrokerStatus>>,
    reconnect_generation: Arc<AtomicU64>,
    generation: u64,
) {
    tokio::spawn(async move {
        time::sleep(Duration::from_secs(3)).await;
        complete_reconnect_grace(&status, &reconnect_generation, generation);
    });
}

fn complete_reconnect_grace(
    status: &Arc<RwLock<CodexBrokerStatus>>,
    reconnect_generation: &AtomicU64,
    generation: u64,
) {
    let mut current = status.write().unwrap();
    if reconnect_generation.load(Ordering::Acquire) == generation
        && current.phase == CodexBrokerPhase::Reconnecting
    {
        current.phase = CodexBrokerPhase::WaitingForClient;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_json_rpc, compatible_codex_versions, compatible_schema_sha256,
        complete_reconnect_grace, extract_command_approval_body, schema_is_compatible,
        select_codex_executable, std_mpsc, try_acquire_client_slot, ApprovalArbiter,
        BrokerDirection, BrokerRuntimeArgs, CodexApprovalResponseOutcome, CodexBrokerEvent,
        CodexBrokerPhase, CodexBrokerStatus, JsonRpcKind, MAX_CODEX_CLIENTS,
        SUPPORTED_CODEX_VERSION, SUPPORTED_SCHEMA_SHA256,
    };
    use futures_util::{SinkExt, StreamExt};
    use serde_json::Value;
    use tokio::{net::TcpListener, sync::oneshot, time};
    use tokio_tungstenite::{
        accept_async, connect_async,
        tungstenite::{client::IntoClientRequest, protocol::Message},
    };

    #[test]
    fn approval_timeout_cancels_queued_command_before_execution() {
        let state = Arc::new(super::ApprovalResponseState::default());
        let (reply_tx, reply_rx) = std_mpsc::channel();
        let (route, mut commands) = tokio::sync::mpsc::unbounded_channel();
        route
            .send(super::ApprovalResponseCommand {
                request_id: serde_json::json!(1),
                decision: serde_json::json!("accept"),
                reply: reply_tx,
                state: state.clone(),
            })
            .unwrap();

        assert!(super::wait_for_approval_response(reply_rx, &state, Duration::ZERO).is_err());
        let command = commands.try_recv().unwrap();
        assert!(!command.state.try_start());
    }

    #[test]
    fn approval_timeout_waits_for_execution_that_already_started() {
        let state = Arc::new(super::ApprovalResponseState::default());
        assert!(state.try_start());
        let (reply_tx, reply_rx) = std_mpsc::channel();
        let (result_tx, result_rx) = std_mpsc::channel();
        let waiter = std::thread::spawn(move || {
            result_tx
                .send(super::wait_for_approval_response(
                    reply_rx,
                    &state,
                    Duration::ZERO,
                ))
                .unwrap();
        });

        assert!(matches!(
            result_rx.recv_timeout(Duration::from_millis(20)),
            Err(std_mpsc::RecvTimeoutError::Timeout)
        ));
        reply_tx
            .send(CodexApprovalResponseOutcome::Accepted)
            .unwrap();
        assert_eq!(
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap(),
            CodexApprovalResponseOutcome::Accepted
        );
        waiter.join().unwrap();
    }

    #[test]
    fn approval_start_and_cancellation_have_only_one_winner() {
        for _ in 0..32 {
            let state = Arc::new(super::ApprovalResponseState::default());
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let executor_state = state.clone();
            let executor_barrier = barrier.clone();
            let executor = std::thread::spawn(move || {
                executor_barrier.wait();
                executor_state.try_start()
            });
            barrier.wait();
            let cancelled = state.try_cancel();
            let started = executor.join().unwrap();
            assert_ne!(cancelled, started);
            assert!(!state.try_start());
            assert!(!state.try_cancel());
        }
    }

    #[test]
    fn hud_claim_wins_and_late_cli_response_is_rejected() {
        let mut arbiter = ApprovalArbiter::default();
        let id = serde_json::json!(17);
        arbiter.observe_request(&id);

        assert_eq!(arbiter.claim(&id), CodexApprovalResponseOutcome::Accepted);
        assert_eq!(
            arbiter.claim(&id),
            CodexApprovalResponseOutcome::AlreadyResolved
        );
    }

    #[test]
    fn cli_claim_wins_and_late_hud_response_is_rejected() {
        let mut arbiter = ApprovalArbiter::default();
        let id = serde_json::json!("approval-1");
        arbiter.observe_request(&id);

        assert_eq!(arbiter.claim(&id), CodexApprovalResponseOutcome::Accepted);
        assert_eq!(
            arbiter.claim(&id),
            CodexApprovalResponseOutcome::AlreadyResolved
        );
    }

    #[test]
    fn app_server_resolution_closes_the_race_and_id_reuse_starts_a_new_one() {
        let mut arbiter = ApprovalArbiter::default();
        let id = serde_json::json!(3);
        arbiter.observe_request(&id);
        arbiter.observe_resolved(&id);
        assert_eq!(
            arbiter.claim(&id),
            CodexApprovalResponseOutcome::AlreadyResolved
        );

        arbiter.observe_request(&id);
        assert_eq!(arbiter.claim(&id), CodexApprovalResponseOutcome::Accepted);
    }

    #[test]
    fn compatibility_gate_accepts_only_verified_releases() {
        assert_eq!(
            compatible_schema_sha256(SUPPORTED_CODEX_VERSION),
            Some(SUPPORTED_SCHEMA_SHA256)
        );
        assert_eq!(
            compatible_schema_sha256("codex-cli 0.151.0"),
            Some("31AE67BEB2C94CC9509F6A71968600062DC8C6D7FE45437ED3A9129838F4D2D9")
        );
        assert_eq!(
            compatible_schema_sha256("codex-cli 0.150.1"),
            Some("E9BAD0A20736E7D3ABA18C0F04BEF59856FB212AE21049FE17D786682203CFAE")
        );
        assert_eq!(
            compatible_schema_sha256("codex-cli 0.149.1"),
            Some("4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91")
        );
        assert_eq!(
            compatible_schema_sha256("codex-cli 0.149.0"),
            Some("4F4A8D8F53F971B97F818639F58C8D26BB68BFCDFA2D2F20572CB97E6761AB91")
        );
        assert_eq!(
            compatible_schema_sha256("codex-cli 0.147.0"),
            Some("BABFD5C98CD978DD858B4762CDFBC9FBA941E1A0E4053DE0050E4082AE1F075A")
        );
        assert_eq!(
            compatible_schema_sha256("codex-cli 0.146.0"),
            Some("D3992FEC1398AFDBEC658DA2C720C6993FBF3C1CE4900785694D2196679EDDFC")
        );
        assert_eq!(compatible_schema_sha256("codex-cli 0.148.0"), None);
        assert_eq!(compatible_schema_sha256("codex-cli 0.149.2"), None);
        assert_eq!(compatible_schema_sha256("codex-cli 0.145.0"), None);
        assert_eq!(
            compatible_codex_versions(),
            "codex-cli 0.153.2, codex-cli 0.151.0, codex-cli 0.150.1, codex-cli 0.149.1, codex-cli 0.149.0, codex-cli 0.147.0, codex-cli 0.146.0"
        );
    }

    #[test]
    fn optional_version_check_still_requires_a_verified_schema() {
        assert!(schema_is_compatible(
            "codex-cli 0.149.2",
            SUPPORTED_SCHEMA_SHA256,
            false
        ));
        assert!(!schema_is_compatible(
            "codex-cli 0.149.2",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            false
        ));
        assert!(!schema_is_compatible(
            "codex-cli 0.149.2",
            SUPPORTED_SCHEMA_SHA256,
            true
        ));
        assert!(schema_is_compatible(
            SUPPORTED_CODEX_VERSION,
            SUPPORTED_SCHEMA_SHA256,
            true
        ));
        assert!(!schema_is_compatible(
            "codex-cli 0.150.1",
            SUPPORTED_SCHEMA_SHA256,
            true
        ));
        assert!(!schema_is_compatible(
            "codex-cli 0.147.0",
            SUPPORTED_SCHEMA_SHA256,
            true
        ));
    }

    #[test]
    fn json_rpc_metadata_extracts_structured_item_type() {
        let metadata = classify_json_rpc(
            r#"{"jsonrpc":"2.0","method":"item/started","params":{"threadId":"thread-a","turnId":"turn-a","item":{"id":"item-a","type":"webSearch","query":"secret"}}}"#,
        );

        assert_eq!(metadata.item_id.as_deref(), Some("item-a"));
        assert_eq!(metadata.item_type.as_deref(), Some("webSearch"));
    }

    /// The real `item/commandExecution/requestApproval` request captured in
    /// `docs/codex-approval-proxy-gate-results.md` §5. Also exercises §5.1:
    /// `availableDecisions` mixes plain strings and an object variant, and
    /// must round-trip through extraction with values unchanged -- never
    /// reconstructed.
    const KO2_REQUEST_APPROVAL_JSON: &str = r#"{
        "id": 0,
        "method": "item/commandExecution/requestApproval",
        "params": {
            "availableDecisions": [
                "accept",
                { "acceptWithExecpolicyAmendment": { "execpolicy_amendment": ["mkdir"] } },
                "cancel"
            ],
            "command": "\"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -Command 'mkdir ko2-test'",
            "commandActions": [ { "command": "mkdir ko2-test", "type": "unknown" } ],
            "cwd": "C:\\01.keyboards\\OriginalKeyboards\\02.SW\\Keylink-Studio",
            "environmentId": "local",
            "itemId": "exec-882ac982-...",
            "kind": "command",
            "proposedExecpolicyAmendment": ["mkdir"],
            "reason": "ワークスペース内に ko2-test ディレクトリを作成してよいですか？",
            "startedAtMs": 1788429792762,
            "threadId": "01a066b8-5269-71b2-9c8a-d7e64a8302a1",
            "turnId": "01a066b8-e33a-7861-8334-256907f36ccc"
        }
    }"#;

    #[test]
    fn extracts_command_approval_body_from_the_real_ko2_request() {
        let body = extract_command_approval_body(KO2_REQUEST_APPROVAL_JSON)
            .expect("well-formed requestApproval body");

        assert_eq!(body.command_actions, vec!["mkdir ko2-test".to_string()]);
        assert_eq!(
            body.command.as_deref(),
            Some(
                "\"C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -Command 'mkdir ko2-test'"
            )
        );
        assert_eq!(
            body.reason.as_deref(),
            Some("ワークスペース内に ko2-test ディレクトリを作成してよいですか？")
        );
        assert_eq!(
            body.cwd.as_deref(),
            Some("C:\\01.keyboards\\OriginalKeyboards\\02.SW\\Keylink-Studio")
        );
        assert_eq!(body.kind.as_deref(), Some("command"));
        assert_eq!(
            body.thread_id.as_deref(),
            Some("01a066b8-5269-71b2-9c8a-d7e64a8302a1")
        );
        assert_eq!(
            body.turn_id.as_deref(),
            Some("01a066b8-e33a-7861-8334-256907f36ccc")
        );
        assert_eq!(body.item_id.as_deref(), Some("exec-882ac982-..."));

        // §5.1: the set is 3 elements, mixing a bare string, an object
        // variant, and another bare string. Extraction must keep each
        // element exactly as received.
        assert_eq!(body.available_decisions.len(), 3);
        assert_eq!(
            body.available_decisions[0],
            Value::String("accept".to_string())
        );
        assert_eq!(
            body.available_decisions[1],
            serde_json::json!({"acceptWithExecpolicyAmendment": {"execpolicy_amendment": ["mkdir"]}})
        );
        assert_eq!(
            body.available_decisions[2],
            Value::String("cancel".to_string())
        );
    }

    #[test]
    fn extract_command_approval_body_rejects_non_matching_shapes() {
        assert!(extract_command_approval_body("not json").is_none());
        assert!(extract_command_approval_body(r#"{"id":1,"method":"other"}"#).is_none());
        assert!(extract_command_approval_body(r#"{"jsonrpc":"2.0"}"#).is_none());
    }

    #[test]
    fn emit_message_metadata_carries_the_body_only_for_request_approval() {
        let (tx, rx) = std_mpsc::channel();
        let approval = Message::Text(KO2_REQUEST_APPROVAL_JSON.into());
        super::emit_message_metadata(
            &tx,
            "connection-1",
            BrokerDirection::AppServerToCli,
            &approval,
        );
        let first = rx.recv().expect("body event sent first");
        match first {
            CodexBrokerEvent::ApprovalRequestBody {
                connection_id,
                request_id,
                body,
            } => {
                assert_eq!(connection_id, "connection-1");
                assert_eq!(request_id, Value::from(0));
                assert_eq!(body.command_actions, vec!["mkdir ko2-test".to_string()]);
            }
            other => panic!("expected ApprovalRequestBody, got {other:?}"),
        }
        let second = rx.recv().expect("message event sent second");
        assert!(matches!(second, CodexBrokerEvent::Message { .. }));
        assert!(rx.try_recv().is_err());

        // A high-frequency, unrelated frame must not produce a body event.
        let unrelated = Message::Text(
            r#"{"jsonrpc":"2.0","method":"item/started","params":{"threadId":"t","turnId":"u","item":{"id":"i","type":"commandExecution"}}}"#
                .into(),
        );
        super::emit_message_metadata(
            &tx,
            "connection-1",
            BrokerDirection::AppServerToCli,
            &unrelated,
        );
        let only = rx.recv().expect("message event sent");
        assert!(matches!(only, CodexBrokerEvent::Message { .. }));
        assert!(rx.try_recv().is_err());

        // The same request text arriving in the CLI->AppServer direction
        // (i.e. not the request itself, but hypothetically mis-routed)
        // must not be treated as a body-bearing frame either.
        super::emit_message_metadata(
            &tx,
            "connection-1",
            BrokerDirection::CliToAppServer,
            &approval,
        );
        let cli_direction = rx.recv().expect("message event sent");
        assert!(matches!(cli_direction, CodexBrokerEvent::Message { .. }));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn broker_accepts_eight_slots_and_rejects_the_ninth() {
        let count = AtomicUsize::new(0);
        for expected in 1..=MAX_CODEX_CLIENTS {
            assert!(try_acquire_client_slot(&count));
            assert_eq!(count.load(Ordering::Acquire), expected);
        }
        assert!(!try_acquire_client_slot(&count));
        assert_eq!(count.load(Ordering::Acquire), MAX_CODEX_CLIENTS);
    }

    #[test]
    fn client_launch_info_reservation_counts_pending_connected_and_reconnecting() {
        let tokens = (0..MAX_CODEX_CLIENTS)
            .map(|index| format!("capability-{index}"))
            .collect::<Vec<_>>();
        let token_refs = tokens.iter().map(String::as_str).collect::<Vec<_>>();
        let launches = test_managed_launches(&token_refs);
        let mut launches = launches.lock().unwrap();
        launches.entries[1].state = super::ManagedCredentialState::Connected;
        launches.entries[2].state = super::ManagedCredentialState::Reconnecting;

        assert_eq!(launches.active_count(), MAX_CODEX_CLIENTS);
        assert!(!launches.can_issue("codex-existing"));
        assert!(!launches.can_issue("codex-ninth"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_keeps_two_authenticated_connections_isolated() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = upstream_listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let Ok(mut socket) = accept_async(socket).await else {
                        return;
                    };
                    while let Some(Ok(message)) = socket.next().await {
                        let closed = message.is_close();
                        if socket.send(message).await.is_err() || closed {
                            break;
                        }
                    }
                });
            }
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let status = Arc::new(RwLock::new(CodexBrokerStatus::default()));
        let broker_status = status.clone();
        let broker = tokio::spawn(super::run_broker(
            listener,
            shutdown_rx,
            BrokerRuntimeArgs {
                upstream_url: format!("ws://{upstream_addr}"),
                app_server_token: "upstream-token".to_string(),
                upstream_timeout: Duration::from_secs(2),
                event_tx,
                status: broker_status,
                managed_launches: test_managed_launches(&["client-token-a", "client-token-b"]),
                approval_routes: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            },
        ));

        let connect = |name: &'static str| async move {
            let mut request = format!("ws://{broker_addr}").into_client_request().unwrap();
            request.headers_mut().insert(
                "authorization",
                format!("Bearer client-token-{name}").parse().unwrap(),
            );
            let (socket, _) = connect_async(request).await.unwrap();
            (name, socket)
        };
        let ((_, mut first), (_, mut second)) = tokio::join!(connect("a"), connect("b"));
        first
            .send(Message::Text(
                r#"{"jsonrpc":"2.0","id":1,"method":"first"}"#.into(),
            ))
            .await
            .unwrap();
        second
            .send(Message::Text(
                r#"{"jsonrpc":"2.0","id":1,"method":"second"}"#.into(),
            ))
            .await
            .unwrap();
        assert!(first
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .contains("first"));
        assert!(second
            .next()
            .await
            .unwrap()
            .unwrap()
            .into_text()
            .unwrap()
            .contains("second"));

        let mut connection_ids = std::collections::HashSet::new();
        let mut methods_by_connection = std::collections::HashMap::new();
        while connection_ids.len() < 2 || methods_by_connection.len() < 2 {
            match event_rx.recv_timeout(Duration::from_secs(2)).unwrap() {
                CodexBrokerEvent::ManagedClientConnected { connection_id, .. } => {
                    connection_ids.insert(connection_id);
                }
                CodexBrokerEvent::Message {
                    connection_id,
                    metadata,
                    ..
                } if metadata.kind == JsonRpcKind::Request => {
                    methods_by_connection.insert(connection_id, metadata.method.clone());
                }
                _ => {}
            }
        }
        assert_eq!(methods_by_connection.len(), 2);
        assert!(methods_by_connection
            .values()
            .any(|method| method.as_deref() == Some("first")));
        assert!(methods_by_connection
            .values()
            .any(|method| method.as_deref() == Some("second")));
        assert_eq!(status.read().unwrap().connected_client_count, 2);
        assert!(status.read().unwrap().client_connected);

        first.close(None).await.unwrap();
        time::timeout(Duration::from_secs(2), async {
            loop {
                if status.read().unwrap().connected_client_count == 1 {
                    break;
                }
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(status.read().unwrap().phase, CodexBrokerPhase::Connected);
        assert!(status.read().unwrap().client_connected);

        let _ = shutdown_tx.send(());
        broker.await.unwrap().unwrap();
        time::timeout(Duration::from_secs(2), second.next())
            .await
            .expect("active downstream task did not stop with Broker");
        upstream.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_forwards_only_the_first_cli_or_hud_approval_response() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let (upstream_tx, upstream_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (socket, _) = upstream_listener.accept().await.unwrap();
            upstream_tx
                .send(accept_async(socket).await.unwrap())
                .unwrap();
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (event_tx, event_rx) = std::sync::mpsc::channel();
        let status = Arc::new(RwLock::new(CodexBrokerStatus::default()));
        let approval_routes = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let broker = tokio::spawn(super::run_broker(
            listener,
            shutdown_rx,
            BrokerRuntimeArgs {
                upstream_url: format!("ws://{upstream_addr}"),
                app_server_token: "upstream-token".to_string(),
                upstream_timeout: Duration::from_secs(2),
                event_tx,
                status,
                managed_launches: test_managed_launches(&["client-token"]),
                approval_routes: approval_routes.clone(),
            },
        ));

        let mut request = format!("ws://{broker_addr}").into_client_request().unwrap();
        request
            .headers_mut()
            .insert("authorization", "Bearer client-token".parse().unwrap());
        let (mut cli, _) = connect_async(request).await.unwrap();
        let mut app_server = upstream_rx.await.unwrap();
        let connection_id = loop {
            if let CodexBrokerEvent::ManagedClientConnected { connection_id, .. } =
                event_rx.recv_timeout(Duration::from_secs(2)).unwrap()
            {
                break connection_id;
            }
        };
        let route = approval_routes
            .lock()
            .unwrap()
            .get(&connection_id)
            .cloned()
            .unwrap();

        let request_one = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "item/commandExecution/requestApproval",
            "params": { "availableDecisions": ["accept", "cancel"] }
        });
        app_server
            .send(Message::Text(request_one.to_string().into()))
            .await
            .unwrap();
        cli.next().await.unwrap().unwrap();
        let (hud_reply_tx, hud_reply_rx) = std_mpsc::channel();
        route
            .send(super::ApprovalResponseCommand {
                request_id: serde_json::json!(1),
                decision: serde_json::json!("accept"),
                reply: hud_reply_tx,
                state: Arc::new(super::ApprovalResponseState::default()),
            })
            .unwrap();
        let hud_response: Value =
            serde_json::from_str(app_server.next().await.unwrap().unwrap().to_text().unwrap())
                .unwrap();
        assert_eq!(hud_response["result"]["decision"], "accept");
        assert_eq!(
            hud_reply_rx.recv().unwrap(),
            CodexApprovalResponseOutcome::Accepted
        );
        cli.send(Message::Text(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "result": { "decision": "cancel" }
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        assert!(time::timeout(Duration::from_millis(100), app_server.next())
            .await
            .is_err());

        let request_two = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "item/commandExecution/requestApproval",
            "params": { "availableDecisions": ["accept", "cancel"] }
        });
        app_server
            .send(Message::Text(request_two.to_string().into()))
            .await
            .unwrap();
        cli.next().await.unwrap().unwrap();
        cli.send(Message::Text(
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": { "decision": "cancel" }
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
        let cli_response: Value =
            serde_json::from_str(app_server.next().await.unwrap().unwrap().to_text().unwrap())
                .unwrap();
        assert_eq!(cli_response["result"]["decision"], "cancel");
        let (late_hud_tx, late_hud_rx) = std_mpsc::channel();
        route
            .send(super::ApprovalResponseCommand {
                request_id: serde_json::json!(2),
                decision: serde_json::json!("accept"),
                reply: late_hud_tx,
                state: Arc::new(super::ApprovalResponseState::default()),
            })
            .unwrap();
        assert_eq!(
            late_hud_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            CodexApprovalResponseOutcome::AlreadyResolved
        );
        assert!(time::timeout(Duration::from_millis(100), app_server.next())
            .await
            .is_err());

        // A timed-out queued command must neither reach upstream nor claim
        // the approval: a later valid HUD response can still answer it.
        app_server
            .send(Message::Text(
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 3,
                    "method": "item/commandExecution/requestApproval",
                    "params": { "availableDecisions": ["accept", "cancel"] }
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        cli.next().await.unwrap().unwrap();
        let cancelled_state = Arc::new(super::ApprovalResponseState::default());
        let (cancelled_tx, cancelled_rx) = std_mpsc::channel();
        assert!(
            super::wait_for_approval_response(cancelled_rx, &cancelled_state, Duration::ZERO)
                .is_err()
        );
        route
            .send(super::ApprovalResponseCommand {
                request_id: serde_json::json!(3),
                decision: serde_json::json!("accept"),
                reply: cancelled_tx,
                state: cancelled_state,
            })
            .unwrap();
        let (valid_tx, valid_rx) = std_mpsc::channel();
        route
            .send(super::ApprovalResponseCommand {
                request_id: serde_json::json!(3),
                decision: serde_json::json!("cancel"),
                reply: valid_tx,
                state: Arc::new(super::ApprovalResponseState::default()),
            })
            .unwrap();
        let response = time::timeout(Duration::from_secs(2), app_server.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let response: Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
        assert_eq!(response["id"], 3);
        assert_eq!(response["result"]["decision"], "cancel");
        assert_eq!(
            valid_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            CodexApprovalResponseOutcome::Accepted
        );
        assert!(time::timeout(Duration::from_millis(100), app_server.next())
            .await
            .is_err());

        let _ = shutdown_tx.send(());
        broker.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn broker_rejects_an_unissued_capability_after_eight_managed_clients() {
        let upstream_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream_listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = upstream_listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let Ok(mut socket) = accept_async(socket).await else {
                        return;
                    };
                    while let Some(Ok(message)) = socket.next().await {
                        if message.is_close() {
                            break;
                        }
                    }
                });
            }
        });

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let broker_addr = listener.local_addr().unwrap();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (event_tx, _event_rx) = std::sync::mpsc::channel();
        let status = Arc::new(RwLock::new(CodexBrokerStatus::default()));
        let broker = tokio::spawn(super::run_broker(
            listener,
            shutdown_rx,
            BrokerRuntimeArgs {
                upstream_url: format!("ws://{upstream_addr}"),
                app_server_token: "upstream-token".to_string(),
                upstream_timeout: Duration::from_secs(2),
                event_tx,
                status: status.clone(),
                managed_launches: test_managed_launches(&[
                    "client-token-0",
                    "client-token-1",
                    "client-token-2",
                    "client-token-3",
                    "client-token-4",
                    "client-token-5",
                    "client-token-6",
                    "client-token-7",
                ]),
                approval_routes: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            },
        ));

        let mut clients = Vec::new();
        for index in 0..MAX_CODEX_CLIENTS {
            let mut request = format!("ws://{broker_addr}").into_client_request().unwrap();
            request.headers_mut().insert(
                "authorization",
                format!("Bearer client-token-{index}").parse().unwrap(),
            );
            clients.push(connect_async(request).await.unwrap().0);
        }
        time::timeout(Duration::from_secs(2), async {
            loop {
                if status.read().unwrap().connected_client_count == MAX_CODEX_CLIENTS {
                    break;
                }
                time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();

        let mut ninth_request = format!("ws://{broker_addr}").into_client_request().unwrap();
        ninth_request
            .headers_mut()
            .insert("authorization", "Bearer client-token-8".parse().unwrap());
        let rejection = connect_async(ninth_request)
            .await
            .expect_err("ninth client must be rejected");
        assert!(matches!(
            rejection,
            tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == 401
        ));
        assert_eq!(
            status.read().unwrap().connected_client_count,
            MAX_CODEX_CLIENTS
        );

        let _ = shutdown_tx.send(());
        broker.await.unwrap().unwrap();
        drop(clients);
        upstream.abort();
    }
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicU64, AtomicUsize, Ordering},
            Arc, RwLock,
        },
        time::Duration,
    };

    fn test_managed_launches(
        tokens: &[&str],
    ) -> Arc<std::sync::Mutex<super::ManagedLaunchRegistry>> {
        Arc::new(std::sync::Mutex::new(super::ManagedLaunchRegistry {
            entries: tokens
                .iter()
                .enumerate()
                .map(|(index, token)| super::ManagedCredential {
                    token: (*token).to_string(),
                    token_path: PathBuf::from(format!("test-capability-{index}.token")),
                    terminal_target_id: format!("codex-test-target-{index}"),
                    display_name: "Codex test".to_string(),
                    state: super::ManagedCredentialState::Pending,
                    deadline: None,
                    remove_at: None,
                })
                .collect(),
        }))
    }

    #[test]
    fn stale_reconnect_grace_cannot_finish_a_newer_disconnect() {
        let status = Arc::new(RwLock::new(CodexBrokerStatus {
            phase: CodexBrokerPhase::Reconnecting,
            ..CodexBrokerStatus::default()
        }));
        let generation = AtomicU64::new(2);

        complete_reconnect_grace(&status, &generation, 1);
        assert_eq!(status.read().unwrap().phase, CodexBrokerPhase::Reconnecting);

        complete_reconnect_grace(&status, &generation, 2);
        assert_eq!(
            status.read().unwrap().phase,
            CodexBrokerPhase::WaitingForClient
        );

        status.write().unwrap().phase = CodexBrokerPhase::Reconnecting;
        generation.store(3, Ordering::Release);
        complete_reconnect_grace(&status, &generation, 2);
        assert_eq!(status.read().unwrap().phase, CodexBrokerPhase::Reconnecting);
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_resolution_skips_extensionless_npm_shim() {
        let output = b"C:\\Users\\test\\AppData\\Roaming\\npm\\codex\r\nC:\\Users\\test\\AppData\\Roaming\\npm\\codex.cmd\r\n";

        assert_eq!(
            select_codex_executable(output),
            Some(PathBuf::from(
                r"C:\Users\test\AppData\Roaming\npm\codex.cmd"
            ))
        );
    }
}
