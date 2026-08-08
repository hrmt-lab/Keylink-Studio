use std::{
    io,
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
    task::JoinHandle,
};

use crate::claude_hook_event::{ClaudeHookEvent, ClaudeObserverEvent, ClaudeWrapperExited};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaudeObserverConfig {
    pub endpoint: String,
    pub wrapper_exit_endpoint: String,
    pub bearer_token: String,
    pub launch_id: String,
    pub request_timeout_ms: u64,
}

impl std::fmt::Debug for ClaudeObserverConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ClaudeObserverConfig")
            .field("endpoint", &self.endpoint)
            .field("wrapper_exit_endpoint", &self.wrapper_exit_endpoint)
            .field("bearer_token", &"<redacted>")
            .field("launch_id", &self.launch_id)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClaudeObserverCounters {
    pub received: u64,
    pub accepted: u64,
    pub unauthorized: u64,
    pub malformed: u64,
    pub oversized: u64,
    pub normal_overflow: u64,
    pub priority_overflow: u64,
}

#[derive(Default)]
struct AtomicCounters {
    received: AtomicU64,
    accepted: AtomicU64,
    unauthorized: AtomicU64,
    malformed: AtomicU64,
    oversized: AtomicU64,
    normal_overflow: AtomicU64,
    priority_overflow: AtomicU64,
}

impl AtomicCounters {
    fn snapshot(&self) -> ClaudeObserverCounters {
        ClaudeObserverCounters {
            received: self.received.load(Ordering::Relaxed),
            accepted: self.accepted.load(Ordering::Relaxed),
            unauthorized: self.unauthorized.load(Ordering::Relaxed),
            malformed: self.malformed.load(Ordering::Relaxed),
            oversized: self.oversized.load(Ordering::Relaxed),
            normal_overflow: self.normal_overflow.load(Ordering::Relaxed),
            priority_overflow: self.priority_overflow.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClaudeObserverReceiverOptions {
    pub bind_address: SocketAddr,
    pub launch_id: String,
    pub bearer_token: String,
    pub normal_queue_capacity: usize,
    pub priority_queue_capacity: usize,
    pub helper_request_timeout_ms: u64,
}

impl ClaudeObserverReceiverOptions {
    pub fn loopback(launch_id: impl Into<String>, bearer_token: impl Into<String>) -> Self {
        Self {
            bind_address: SocketAddr::from(([127, 0, 0, 1], 0)),
            launch_id: launch_id.into(),
            bearer_token: bearer_token.into(),
            normal_queue_capacity: 128,
            priority_queue_capacity: 16,
            helper_request_timeout_ms: 500,
        }
    }
}

#[derive(Debug, Error)]
pub enum ClaudeObserverError {
    #[error("invalid Claude observer configuration: {0}")]
    InvalidConfig(String),
    #[error("failed to bind Claude observer receiver: {0}")]
    Bind(#[source] io::Error),
    #[error("Claude observer receiver task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
}

pub struct ClaudeObserverEvents {
    normal_rx: mpsc::Receiver<ClaudeObserverEvent>,
    priority_rx: mpsc::Receiver<ClaudeObserverEvent>,
}

impl ClaudeObserverEvents {
    pub async fn recv(&mut self) -> Option<ClaudeObserverEvent> {
        tokio::select! {
            biased;
            event = self.priority_rx.recv() => event,
            event = self.normal_rx.recv() => event,
        }
    }

    pub fn try_recv(&mut self) -> Result<ClaudeObserverEvent, mpsc::error::TryRecvError> {
        match self.priority_rx.try_recv() {
            Ok(event) => Ok(event),
            Err(mpsc::error::TryRecvError::Empty) => self.normal_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected) => self.normal_rx.try_recv(),
        }
    }
}

pub struct ClaudeObserverReceiver {
    config: ClaudeObserverConfig,
    counters: Arc<AtomicCounters>,
    shutdown_tx: watch::Sender<bool>,
    task: JoinHandle<()>,
}

impl ClaudeObserverReceiver {
    pub async fn bind(
        options: ClaudeObserverReceiverOptions,
    ) -> Result<(Self, ClaudeObserverEvents), ClaudeObserverError> {
        if options.launch_id.is_empty() {
            return Err(ClaudeObserverError::InvalidConfig(
                "launch_id must not be empty".to_string(),
            ));
        }
        if options.bearer_token.len() < 32 {
            return Err(ClaudeObserverError::InvalidConfig(
                "bearer token must contain at least 32 characters".to_string(),
            ));
        }
        if options.normal_queue_capacity == 0 || options.priority_queue_capacity == 0 {
            return Err(ClaudeObserverError::InvalidConfig(
                "queue capacities must be greater than zero".to_string(),
            ));
        }
        if options.helper_request_timeout_ms == 0 {
            return Err(ClaudeObserverError::InvalidConfig(
                "helper timeout must be greater than zero".to_string(),
            ));
        }

        let listener = TcpListener::bind(options.bind_address)
            .await
            .map_err(ClaudeObserverError::Bind)?;
        let address = listener.local_addr().map_err(ClaudeObserverError::Bind)?;
        let base = format!("http://{address}");
        let config = ClaudeObserverConfig {
            endpoint: format!("{base}/hooks"),
            wrapper_exit_endpoint: format!("{base}/wrapper-exit"),
            bearer_token: options.bearer_token.clone(),
            launch_id: options.launch_id.clone(),
            request_timeout_ms: options.helper_request_timeout_ms,
        };
        let (normal_tx, normal_rx) = mpsc::channel(options.normal_queue_capacity);
        let (priority_tx, priority_rx) = mpsc::channel(options.priority_queue_capacity);
        let counters = Arc::new(AtomicCounters::default());
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let state = ReceiverState {
            launch_id: Arc::new(options.launch_id),
            bearer_token: Arc::new(options.bearer_token),
            normal_tx,
            priority_tx,
            counters: counters.clone(),
        };
        let task = tokio::spawn(receiver_loop(listener, state, shutdown_rx));

        Ok((
            Self {
                config,
                counters,
                shutdown_tx,
                task,
            },
            ClaudeObserverEvents {
                normal_rx,
                priority_rx,
            },
        ))
    }

    pub fn config(&self) -> &ClaudeObserverConfig {
        &self.config
    }

    pub fn counters(&self) -> ClaudeObserverCounters {
        self.counters.snapshot()
    }

    pub async fn shutdown(self) -> Result<(), ClaudeObserverError> {
        let _ = self.shutdown_tx.send(true);
        self.task.await?;
        Ok(())
    }
}

#[derive(Clone)]
struct ReceiverState {
    launch_id: Arc<String>,
    bearer_token: Arc<String>,
    normal_tx: mpsc::Sender<ClaudeObserverEvent>,
    priority_tx: mpsc::Sender<ClaudeObserverEvent>,
    counters: Arc<AtomicCounters>,
}

async fn receiver_loop(
    listener: TcpListener,
    state: ReceiverState,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    return;
                }
            }
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else {
                    return;
                };
                tokio::spawn(handle_connection(stream, state.clone()));
            }
        }
    }
}

async fn handle_connection(mut stream: TcpStream, state: ReceiverState) {
    state.counters.received.fetch_add(1, Ordering::Relaxed);
    let request = match read_http_request(&mut stream).await {
        Ok(request) => request,
        Err(RequestError::Malformed) => {
            state.counters.malformed.fetch_add(1, Ordering::Relaxed);
            let _ = write_response(&mut stream, 400).await;
            return;
        }
        Err(RequestError::Oversized) => {
            state.counters.oversized.fetch_add(1, Ordering::Relaxed);
            let _ = write_response(&mut stream, 413).await;
            return;
        }
        Err(RequestError::Io) => return,
    };

    if request.method != "POST" {
        state.counters.malformed.fetch_add(1, Ordering::Relaxed);
        let _ = write_response(&mut stream, 405).await;
        return;
    }
    let expected = format!("Bearer {}", state.bearer_token);
    let authorized = request
        .authorization
        .as_deref()
        .map(|actual| constant_time_equal(actual.as_bytes(), expected.as_bytes()))
        .unwrap_or(false);
    if !authorized {
        state.counters.unauthorized.fetch_add(1, Ordering::Relaxed);
        let _ = write_response(&mut stream, 401).await;
        return;
    }

    let event = match request.path.as_str() {
        "/hooks" => parse_hook_event(&request.body, &state.launch_id),
        "/wrapper-exit" => parse_wrapper_exit(&request.body, &state.launch_id),
        _ => {
            let _ = write_response(&mut stream, 404).await;
            return;
        }
    };
    let Some(event) = event else {
        state.counters.malformed.fetch_add(1, Ordering::Relaxed);
        let _ = write_response(&mut stream, 400).await;
        return;
    };

    let queued = if event.is_priority() {
        state.priority_tx.try_send(event).map_err(|_| {
            state
                .counters
                .priority_overflow
                .fetch_add(1, Ordering::Relaxed)
        })
    } else {
        state.normal_tx.try_send(event).map_err(|_| {
            state
                .counters
                .normal_overflow
                .fetch_add(1, Ordering::Relaxed)
        })
    };
    if queued.is_ok() {
        state.counters.accepted.fetch_add(1, Ordering::Relaxed);
    }
    let _ = write_response(&mut stream, 204).await;
}

fn parse_hook_event(body: &[u8], launch_id: &str) -> Option<ClaudeObserverEvent> {
    let body: Value = serde_json::from_slice(body).ok()?;
    let hook_event_name = body.get("hook_event_name")?.as_str()?.to_string();
    let session_id = body
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(ClaudeObserverEvent::Hook(ClaudeHookEvent {
        launch_id: launch_id.to_string(),
        hook_event_name,
        session_id,
        body,
    }))
}

#[derive(Deserialize)]
struct WrapperExitBody {
    launch_id: String,
    exit_code: i32,
}

fn parse_wrapper_exit(body: &[u8], launch_id: &str) -> Option<ClaudeObserverEvent> {
    let body: WrapperExitBody = serde_json::from_slice(body).ok()?;
    if body.launch_id != launch_id {
        return None;
    }
    Some(ClaudeObserverEvent::WrapperExited(ClaudeWrapperExited {
        launch_id: body.launch_id,
        exit_code: body.exit_code,
    }))
}

struct HttpRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

enum RequestError {
    Malformed,
    Oversized,
    Io,
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, RequestError> {
    let mut bytes = Vec::with_capacity(4096);
    let header_end = loop {
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(RequestError::Oversized);
        }
        if let Some(position) = find_subslice(&bytes, b"\r\n\r\n") {
            break position + 4;
        }
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RequestError::Io)?;
        if read == 0 {
            return Err(RequestError::Malformed);
        }
        bytes.extend_from_slice(&chunk[..read]);
    };

    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Request::new(&mut headers);
    parsed
        .parse(&bytes[..header_end])
        .map_err(|_| RequestError::Malformed)?;
    let method = parsed.method.ok_or(RequestError::Malformed)?.to_string();
    let path = parsed.path.ok_or(RequestError::Malformed)?.to_string();
    let mut authorization = None;
    let mut content_length = None;
    for header in parsed.headers {
        if header.name.eq_ignore_ascii_case("authorization") {
            authorization = Some(
                std::str::from_utf8(header.value)
                    .map_err(|_| RequestError::Malformed)?
                    .to_string(),
            );
        } else if header.name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                std::str::from_utf8(header.value)
                    .map_err(|_| RequestError::Malformed)?
                    .parse::<usize>()
                    .map_err(|_| RequestError::Malformed)?,
            );
        }
    }
    let content_length = content_length.ok_or(RequestError::Malformed)?;
    if content_length > MAX_BODY_BYTES {
        return Err(RequestError::Oversized);
    }
    let required = header_end + content_length;
    while bytes.len() < required {
        let mut chunk = [0_u8; 4096];
        let read = stream
            .read(&mut chunk)
            .await
            .map_err(|_| RequestError::Io)?;
        if read == 0 {
            return Err(RequestError::Malformed);
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() > required + MAX_HEADER_BYTES {
            return Err(RequestError::Oversized);
        }
    }
    Ok(HttpRequest {
        method,
        path,
        authorization,
        body: bytes[header_end..required].to_vec(),
    })
}

async fn write_response(stream: &mut TcpStream, status: u16) -> io::Result<()> {
    let reason = match status {
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Internal Server Error",
    };
    let response =
        format!("HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && left.ct_eq(right).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    #[test]
    fn config_debug_redacts_token() {
        let config = ClaudeObserverConfig {
            endpoint: "http://127.0.0.1/hooks".to_string(),
            wrapper_exit_endpoint: "http://127.0.0.1/wrapper-exit".to_string(),
            bearer_token: "secret-token".to_string(),
            launch_id: "launch".to_string(),
            request_timeout_ms: 500,
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("secret-token"));
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn receiver_accepts_hook_and_wrapper_events() {
        runtime().block_on(async {
            let token = "0123456789abcdef0123456789abcdef";
            let (receiver, mut events) = ClaudeObserverReceiver::bind(
                ClaudeObserverReceiverOptions::loopback("launch-1", token),
            )
            .await
            .unwrap();
            let client = reqwest::Client::new();
            let response = client
                .post(&receiver.config().endpoint)
                .bearer_auth(token)
                .json(&serde_json::json!({
                    "hook_event_name": "SessionStart",
                    "session_id": "session-1",
                    "prompt": "must stay in memory only"
                }))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
            let event = events.recv().await.unwrap();
            let ClaudeObserverEvent::Hook(event) = event else {
                panic!("expected hook event");
            };
            assert_eq!(event.launch_id, "launch-1");
            assert_eq!(event.session_id.as_deref(), Some("session-1"));

            let response = client
                .post(&receiver.config().wrapper_exit_endpoint)
                .bearer_auth(token)
                .json(&serde_json::json!({"launch_id": "launch-1", "exit_code": 7}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
            assert_eq!(
                events.recv().await,
                Some(ClaudeObserverEvent::WrapperExited(ClaudeWrapperExited {
                    launch_id: "launch-1".to_string(),
                    exit_code: 7,
                }))
            );
            assert_eq!(receiver.counters().accepted, 2);
            receiver.shutdown().await.unwrap();
        });
    }

    #[test]
    fn receiver_rejects_wrong_token_without_queueing() {
        runtime().block_on(async {
            let (receiver, mut events) =
                ClaudeObserverReceiver::bind(ClaudeObserverReceiverOptions::loopback(
                    "launch-1",
                    "0123456789abcdef0123456789abcdef",
                ))
                .await
                .unwrap();
            let response = reqwest::Client::new()
                .post(&receiver.config().endpoint)
                .bearer_auth("wrong-token")
                .json(&serde_json::json!({"hook_event_name": "Stop"}))
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
            assert_eq!(events.try_recv(), Err(mpsc::error::TryRecvError::Empty));
            assert_eq!(receiver.counters().unauthorized, 1);
            receiver.shutdown().await.unwrap();
        });
    }

    #[test]
    fn receiver_drops_overflowing_detail_without_blocking_response() {
        runtime().block_on(async {
            let token = "0123456789abcdef0123456789abcdef";
            let mut options = ClaudeObserverReceiverOptions::loopback("launch-1", token);
            options.normal_queue_capacity = 1;
            let (receiver, mut events) = ClaudeObserverReceiver::bind(options).await.unwrap();
            let client = reqwest::Client::new();
            for tool_id in ["tool-1", "tool-2"] {
                let response = client
                    .post(&receiver.config().endpoint)
                    .bearer_auth(token)
                    .json(&serde_json::json!({
                        "hook_event_name": "PreToolUse",
                        "session_id": "session-1",
                        "tool_use_id": tool_id
                    }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
            }
            assert_eq!(receiver.counters().accepted, 1);
            assert_eq!(receiver.counters().normal_overflow, 1);
            assert!(matches!(
                events.recv().await,
                Some(ClaudeObserverEvent::Hook(_))
            ));
            receiver.shutdown().await.unwrap();
        });
    }
}
