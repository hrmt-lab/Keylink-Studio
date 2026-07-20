use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{mpsc as std_mpsc, Arc, Mutex, RwLock},
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

pub const SUPPORTED_CODEX_VERSION: &str = "codex-cli 0.144.6";
pub const SUPPORTED_SCHEMA_SHA256: &str =
    "85EA836927D6CFDD3C68A9BDA17DBA48D2573BBC282AB2D5775A5005E40BC9C3";
const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct CodexBrokerConfig {
    pub codex_executable: Option<PathBuf>,
    pub app_server_port: u16,
    pub broker_port: u16,
    pub startup_timeout: Duration,
    pub shutdown_timeout: Duration,
}

impl Default for CodexBrokerConfig {
    fn default() -> Self {
        Self {
            codex_executable: None,
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
    pub cli_connection_command: Option<String>,
    pub last_error: Option<String>,
}

impl Default for CodexBrokerStatus {
    fn default() -> Self {
        Self {
            phase: CodexBrokerPhase::Stopped,
            app_server_port: None,
            broker_port: None,
            codex_version: None,
            client_connected: false,
            cli_connection_command: None,
            last_error: None,
        }
    }
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
    pub request_id: Option<Value>,
    pub result_thread_id: Option<String>,
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

enum ManagerCommand {
    Start(
        CodexBrokerConfig,
        std_mpsc::Sender<Result<CodexBrokerStatus, CodexBrokerError>>,
    ),
    Stop(std_mpsc::Sender<Result<CodexBrokerStatus, CodexBrokerError>>),
    Shutdown,
}

struct ManagerInner {
    command_tx: mpsc::UnboundedSender<ManagerCommand>,
    event_rx: Mutex<std_mpsc::Receiver<CodexBrokerEvent>>,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
    status: Arc<RwLock<CodexBrokerStatus>>,
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
        let worker = thread::Builder::new()
            .name("codex-broker-manager".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .thread_name("codex-broker-runtime")
                    .build();
                match runtime {
                    Ok(runtime) => {
                        runtime.block_on(manager_loop(command_rx, event_tx, worker_status.clone()))
                    }
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

    pub fn try_recv_event(&self) -> Result<CodexBrokerEvent, std_mpsc::TryRecvError> {
        self.inner.event_rx.lock().unwrap().try_recv()
    }

    pub fn recv_event_timeout(
        &self,
        timeout: Duration,
    ) -> Result<CodexBrokerEvent, std_mpsc::RecvTimeoutError> {
        self.inner.event_rx.lock().unwrap().recv_timeout(timeout)
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
    broker_shutdown: Option<oneshot::Sender<()>>,
    broker_task: Option<JoinHandle<Result<(), CodexBrokerError>>>,
    _secrets: TempDir,
    config: CodexBrokerConfig,
}

async fn manager_loop(
    mut command_rx: mpsc::UnboundedReceiver<ManagerCommand>,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
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
                            match start_session(config, event_tx.clone(), status.clone()).await {
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
                            set_phase(&status, CodexBrokerPhase::Stopping, false, None);
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
                    ManagerCommand::Shutdown => break,
                }
            }
            _ = health_tick.tick(), if session.is_some() => {
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
) -> Result<Session, CodexBrokerError> {
    config.validate()?;
    ensure_port_available(config.app_server_port, "App Server").await?;
    ensure_port_available(config.broker_port, "Broker").await?;

    let executable = resolve_codex_executable(config.codex_executable.as_deref()).await?;
    let secrets = tempfile::Builder::new()
        .prefix("keylink-codex-")
        .tempdir()
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let version =
        verify_codex_and_schema(&executable, secrets.path(), config.startup_timeout).await?;

    let app_server_token = generate_token()?;
    let broker_token = loop {
        let candidate = generate_token()?;
        if !constant_time_eq(&candidate, &app_server_token) {
            break candidate;
        }
    };
    let app_server_token_path = secrets.path().join("app-server.token");
    let broker_token_path = secrets.path().join("broker.token");
    write_private_token(&app_server_token_path, &app_server_token)?;
    write_private_token(&broker_token_path, &broker_token)?;
    let cli_connection_command =
        make_cli_connection_command(&executable, &broker_token_path, config.broker_port);

    let app_server_url = format!("ws://127.0.0.1:{}", config.app_server_port);
    let mut command = Command::new(&executable);
    command
        .arg("app-server")
        .arg("--listen")
        .arg(&app_server_url)
        .arg("--ws-auth")
        .arg("capability-token")
        .arg("--ws-token-file")
        .arg(&app_server_token_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_child_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| CodexBrokerError::AppServer(error.to_string()))?;
    let child_stdin = child.stdin.take();
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
    let broker_task = tokio::spawn(async move {
        run_broker(
            listener,
            shutdown_rx,
            BrokerRuntimeArgs {
                upstream_url: app_server_url,
                client_token: broker_token,
                app_server_token,
                upstream_timeout: broker_config.startup_timeout,
                event_tx: broker_events,
                status: broker_status,
            },
        )
        .await
    });

    {
        let mut current = status.write().unwrap();
        current.phase = CodexBrokerPhase::WaitingForClient;
        current.codex_version = Some(version);
        current.client_connected = false;
        current.cli_connection_command = Some(cli_connection_command);
        current.last_error = None;
    }
    let _ = event_tx.send(CodexBrokerEvent::Ready {
        app_server_port: config.app_server_port,
        broker_port: config.broker_port,
    });
    Ok(Session {
        child,
        child_stdin,
        broker_shutdown: Some(broker_shutdown),
        broker_task: Some(broker_task),
        _secrets: secrets,
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
}

async fn terminate_child(child: &mut Child, stdin: Option<ChildStdin>, timeout: Duration) {
    drop(stdin);
    if time::timeout(timeout, child.wait()).await.is_err() {
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
}

async fn run_broker(
    listener: TcpListener,
    mut shutdown_rx: oneshot::Receiver<()>,
    args: BrokerRuntimeArgs,
) -> Result<(), CodexBrokerError> {
    let active = Arc::new(std::sync::atomic::AtomicBool::new(false));
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
                    client_token: args.client_token.clone(),
                    app_server_token: args.app_server_token.clone(),
                    upstream_timeout: args.upstream_timeout,
                    active: active.clone(),
                    event_tx: args.event_tx.clone(),
                    status: args.status.clone(),
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
    client_token: String,
    app_server_token: String,
    upstream_timeout: Duration,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
}

struct ConnectionArgs {
    upstream_url: String,
    client_token: String,
    app_server_token: String,
    upstream_timeout: Duration,
    active: Arc<std::sync::atomic::AtomicBool>,
    event_tx: std_mpsc::Sender<CodexBrokerEvent>,
    status: Arc<RwLock<CodexBrokerStatus>>,
    shutdown_rx: broadcast::Receiver<()>,
}

struct ActiveConnectionGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for ActiveConnectionGuard {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
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
    if !constant_time_eq(
        request.bearer_token.as_deref().unwrap_or(""),
        &args.client_token,
    ) {
        let _ = args.event_tx.send(CodexBrokerEvent::DownstreamAuthRejected);
        reject_upgrade(&mut downstream, "401 Unauthorized", "Unauthorized").await;
        return Ok(());
    }
    if args
        .active
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        )
        .is_err()
    {
        let _ = args
            .event_tx
            .send(CodexBrokerEvent::AdditionalClientRejected);
        reject_upgrade(
            &mut downstream,
            "409 Conflict",
            "A CLI client is already connected",
        )
        .await;
        return Ok(());
    }
    let _active_guard = ActiveConnectionGuard(args.active.clone());

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
                    reject_upgrade(&mut downstream, "502 Bad Gateway", "App Server connection failed").await;
                    return Ok(());
                }
            }
        }
    };

    accept_upgrade(&mut downstream, &request.websocket_key).await?;
    let downstream = WebSocketStream::from_raw_socket(downstream, Role::Server, None).await;
    let connection_id = random_identifier()?;
    set_phase(&args.status, CodexBrokerPhase::Connected, true, None);
    let _ = args.event_tx.send(CodexBrokerEvent::ClientConnected {
        connection_id: connection_id.clone(),
    });
    let origin = forward_messages(
        downstream,
        upstream,
        &connection_id,
        &args.event_tx,
        &mut args.shutdown_rx,
    )
    .await;
    let reconnecting = origin == "cli";
    set_phase(
        &args.status,
        if reconnecting {
            CodexBrokerPhase::Reconnecting
        } else {
            CodexBrokerPhase::WaitingForClient
        },
        false,
        None,
    );
    if reconnecting {
        schedule_waiting_for_client_after_grace(args.status.clone());
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
) -> &'static str
where
    Upstream: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (mut downstream_write, mut downstream_read) = downstream.split();
    let (mut upstream_write, mut upstream_read) = upstream.split();
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
                emit_message_metadata(event_tx, connection_id, BrokerDirection::CliToAppServer, &message);
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
                emit_message_metadata(event_tx, connection_id, BrokerDirection::AppServerToCli, &message);
                let closed = message.is_close();
                if downstream_write.send(message).await.is_err() || closed {
                    return "app_server";
                }
            }
        }
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
            request_id: None,
            result_thread_id: None,
            turn_status: None,
            turn_has_error: false,
            will_retry: None,
            batch_count: None,
        },
        _ => return,
    };
    let _ = event_tx.send(CodexBrokerEvent::Message {
        connection_id: connection_id.to_string(),
        direction,
        metadata: Box::new(metadata),
    });
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
            request_id: None,
            result_thread_id: None,
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
        request_id: scalar_at(&value, &["params", "requestId"]),
        result_thread_id: string_at(&value, &["result", "thread", "id"]),
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
        request_id: None,
        result_thread_id: None,
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
) -> Result<String, CodexBrokerError> {
    let version_output = time::timeout(timeout, Command::new(executable).arg("--version").output())
        .await
        .map_err(|_| CodexBrokerError::Preflight("codex --version timed out".to_string()))?
        .map_err(|error| CodexBrokerError::Preflight(error.to_string()))?;
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    if !version_output.status.success() || version != SUPPORTED_CODEX_VERSION {
        return Err(CodexBrokerError::Preflight(format!(
            "requires {SUPPORTED_CODEX_VERSION}; detected {version:?}"
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
    if actual_hash != SUPPORTED_SCHEMA_SHA256 {
        return Err(CodexBrokerError::Preflight(format!(
            "unsupported Codex App Server Schema SHA-256: {actual_hash}"
        )));
    }
    Ok(version)
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

fn make_cli_connection_command(executable: &Path, token_path: &Path, broker_port: u16) -> String {
    fn quote(value: &Path) -> String {
        let value = value.to_string_lossy();
        let value = if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            format!(r"\\{unc}")
        } else {
            value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
        };
        format!("'{}'", value.replace('\'', "''"))
    }
    format!(
        "$env:KEYLINK_CODEX_BROKER_TOKEN = Get-Content -LiteralPath {} -Raw; & {} --remote ws://127.0.0.1:{} --remote-auth-token-env KEYLINK_CODEX_BROKER_TOKEN",
        quote(token_path),
        quote(executable),
        broker_port
    )
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
    current.cli_connection_command = None;
    current.last_error = None;
}

fn set_stopped_status(status: &Arc<RwLock<CodexBrokerStatus>>) {
    *status.write().unwrap() = CodexBrokerStatus::default();
}

fn set_error_status(status: &Arc<RwLock<CodexBrokerStatus>>, detail: String) {
    let mut current = status.write().unwrap();
    current.phase = CodexBrokerPhase::Error;
    current.client_connected = false;
    current.last_error = Some(detail);
}

fn set_phase(
    status: &Arc<RwLock<CodexBrokerStatus>>,
    phase: CodexBrokerPhase,
    connected: bool,
    error: Option<String>,
) {
    let mut current = status.write().unwrap();
    current.phase = phase;
    current.client_connected = connected;
    current.last_error = error;
}

fn schedule_waiting_for_client_after_grace(status: Arc<RwLock<CodexBrokerStatus>>) {
    tokio::spawn(async move {
        time::sleep(Duration::from_secs(3)).await;
        let mut current = status.write().unwrap();
        if current.phase == CodexBrokerPhase::Reconnecting {
            current.phase = CodexBrokerPhase::WaitingForClient;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{make_cli_connection_command, select_codex_executable};
    use std::path::{Path, PathBuf};

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

    #[test]
    fn cli_command_removes_windows_verbatim_path_prefixes() {
        let command = make_cli_connection_command(
            Path::new(r"\\?\C:\Users\test\AppData\Roaming\npm\codex.cmd"),
            Path::new(r"\\?\C:\Users\test\AppData\Local\Temp\broker.token"),
            4501,
        );

        assert!(command.contains(r"& 'C:\Users\test\AppData\Roaming\npm\codex.cmd'"));
        assert!(command
            .contains(r"Get-Content -LiteralPath 'C:\Users\test\AppData\Local\Temp\broker.token'"));
        assert!(!command.contains(r"\\?\"));
    }
}
