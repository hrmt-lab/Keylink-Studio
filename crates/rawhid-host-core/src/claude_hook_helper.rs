use std::{
    ffi::OsString,
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
    thread,
    time::Duration,
};

use serde::Serialize;

use crate::claude_observer::ClaudeObserverConfig;

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_OBSERVER_BYTES: usize = 64 * 1024;
const RETRY_DELAY: Duration = Duration::from_millis(100);

pub fn run_claude_hook_helper(args: impl IntoIterator<Item = OsString>) -> ExitCode {
    let args = args.into_iter().collect::<Vec<_>>();
    let Some(command) = args.first().and_then(|value| value.to_str()) else {
        return ExitCode::SUCCESS;
    };
    match command {
        "forward" => forward(&args[1..]),
        "wrapper-exit" => wrapper_exit(&args[1..]),
        _ => ExitCode::SUCCESS,
    }
}

fn forward(args: &[OsString]) -> ExitCode {
    let Some(observer_path) = option_value(args, "--observer").map(PathBuf::from) else {
        return ExitCode::SUCCESS;
    };
    let Ok(observer) = read_observer_file(&observer_path) else {
        return ExitCode::SUCCESS;
    };
    let Ok(body) = read_limited(io::stdin().lock(), MAX_BODY_BYTES) else {
        return ExitCode::SUCCESS;
    };
    post_with_retry(&observer, &observer.endpoint, body);
    ExitCode::SUCCESS
}

fn wrapper_exit(args: &[OsString]) -> ExitCode {
    if !has_flag(args, "--observer-stdin") {
        return ExitCode::SUCCESS;
    }
    let Some(exit_code) = option_value(args, "--exit-code").and_then(|value| value.parse().ok())
    else {
        return ExitCode::SUCCESS;
    };
    let Ok(observer_bytes) = read_limited(io::stdin().lock(), MAX_OBSERVER_BYTES) else {
        return ExitCode::SUCCESS;
    };
    let Ok(observer) = serde_json::from_slice::<ClaudeObserverConfig>(&observer_bytes) else {
        return ExitCode::SUCCESS;
    };
    #[derive(Serialize)]
    struct WrapperExitBody<'a> {
        launch_id: &'a str,
        exit_code: i32,
    }
    let Ok(body) = serde_json::to_vec(&WrapperExitBody {
        launch_id: &observer.launch_id,
        exit_code,
    }) else {
        return ExitCode::SUCCESS;
    };
    post_with_retry(&observer, &observer.wrapper_exit_endpoint, body);
    ExitCode::SUCCESS
}

fn post_with_retry(observer: &ClaudeObserverConfig, endpoint: &str, body: Vec<u8>) {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(observer.request_timeout_ms.max(1)))
        .build()
    else {
        return;
    };
    for attempt in 0..2 {
        let result = client
            .post(endpoint)
            .bearer_auth(&observer.bearer_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .send();
        if result
            .as_ref()
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        if attempt == 0 {
            thread::sleep(RETRY_DELAY);
        }
    }
}

fn read_observer_file(path: &std::path::Path) -> Result<ClaudeObserverConfig, ()> {
    let file = std::fs::File::open(path).map_err(|_| ())?;
    let bytes = read_limited(file, MAX_OBSERVER_BYTES)?;
    serde_json::from_slice(&bytes).map_err(|_| ())
}

fn read_limited(mut reader: impl Read, limit: usize) -> Result<Vec<u8>, ()> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > limit {
        return Err(());
    }
    Ok(bytes)
}

fn option_value<'a>(args: &'a [OsString], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find_map(|pair| (pair[0] == name).then(|| pair[1].to_str()).flatten())
}

fn has_flag(args: &[OsString], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, sync::mpsc};

    #[test]
    fn limited_reader_rejects_oversized_input() {
        assert_eq!(read_limited(&b"1234"[..], 3), Err(()));
        assert_eq!(read_limited(&b"123"[..], 3), Ok(b"123".to_vec()));
    }

    #[test]
    fn unknown_or_incomplete_commands_are_observation_only_successes() {
        assert_eq!(
            run_claude_hook_helper([OsString::from("unknown")]),
            ExitCode::SUCCESS
        );
        assert_eq!(
            run_claude_hook_helper([OsString::from("forward")]),
            ExitCode::SUCCESS
        );
    }

    #[test]
    fn http_forward_retries_once_and_keeps_token_out_of_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_tx, request_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let request = read_http_message(&mut stream);
                if attempt == 0 {
                    stream
                        .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                } else {
                    request_tx.send(request).unwrap();
                    stream
                        .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                        .unwrap();
                }
            }
        });
        let config = ClaudeObserverConfig {
            endpoint: format!("http://{address}/hooks"),
            wrapper_exit_endpoint: format!("http://{address}/wrapper-exit"),
            bearer_token: "0123456789abcdef0123456789abcdef".to_string(),
            launch_id: "launch-1".to_string(),
            request_timeout_ms: 500,
        };
        post_with_retry(
            &config,
            &config.endpoint,
            br#"{"hook_event_name":"SessionStart"}"#.to_vec(),
        );
        let request = request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: bearer 0123456789abcdef0123456789abcdef"));
        let body = request.split("\r\n\r\n").nth(1).unwrap();
        assert_eq!(body, r#"{"hook_event_name":"SessionStart"}"#);
        assert!(!body.contains(&config.bearer_token));
        server.join().unwrap();
    }

    fn read_http_message(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read as _;

        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            bytes.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let header_end = header_end + 4;
                let headers = String::from_utf8_lossy(&bytes[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length: ")
                            .or_else(|| line.strip_prefix("Content-Length: "))
                    })
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                if bytes.len() >= header_end + content_length {
                    return String::from_utf8(bytes[..header_end + content_length].to_vec())
                        .unwrap();
                }
            }
        }
    }
}
