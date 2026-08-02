use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rawhid_host_core::{CodexClientLaunchInfo, CodexLaunchEnvironment, CodexLauncherConfig};
use serde::Serialize;

const BROKER_TOKEN_ENV: &str = "KEYLINK_CODEX_BROKER_TOKEN";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WslDistribution {
    pub name: String,
    pub version: u8,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CodexLaunchResult {
    pub environment: CodexLaunchEnvironment,
    pub project_directory: String,
}

pub fn list_wsl_distributions() -> Result<Vec<WslDistribution>, String> {
    #[cfg(windows)]
    {
        let output = Command::new("wsl.exe")
            .args(["--list", "--verbose"])
            .output()
            .map_err(|error| format!("WSLを起動できません: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "WSLディストリビューションを取得できません: {}",
                decode_windows_output(&output.stderr).trim()
            ));
        }
        Ok(parse_wsl_distributions(&decode_windows_output(
            &output.stdout,
        )))
    }
    #[cfg(not(windows))]
    {
        Err("CodexランチャーはWindows版Keylink Studioでのみ利用できます".to_string())
    }
}

pub fn launch(
    config: &CodexLauncherConfig,
    connection: &CodexClientLaunchInfo,
) -> Result<CodexLaunchResult, String> {
    validate(config)?;
    validate_connection(connection)?;
    match (&config.environment, &connection.runtime) {
        (CodexLaunchEnvironment::Windows, rawhid_host_core::CodexAppServerRuntime::Windows)
        | (CodexLaunchEnvironment::Wsl, rawhid_host_core::CodexAppServerRuntime::Wsl { .. }) => {}
        _ => {
            return Err(
                "Codex連携の実行環境が設定と一致しません。連携を停止してから再起動してください"
                    .to_string(),
            )
        }
    }
    let (script, title, project_directory) = match config.environment {
        CodexLaunchEnvironment::Windows => {
            let project = required_value(
                config.windows_project_directory.as_deref(),
                "Windowsのプロジェクトディレクトリを選択してください",
            )?;
            validate_windows_project(project)?;
            (
                windows_bootstrap_script(project, connection),
                terminal_title(project),
                project.to_string(),
            )
        }
        CodexLaunchEnvironment::Wsl => {
            let distro = required_value(
                config.wsl_distribution.as_deref(),
                "WSLディストリビューションを選択してください",
            )?;
            let project = required_value(
                config.wsl_project_directory.as_deref(),
                "WSLのプロジェクトディレクトリを入力してください",
            )?;
            let executable = required_value(
                Some(config.wsl_executable.as_str()),
                "WSL側のCodex実行ファイルを入力してください",
            )?;
            validate_shell_value(distro, "WSLディストリビューション")?;
            validate_wsl_project(project)?;
            validate_shell_value(executable, "WSL側のCodex実行ファイル")?;
            ensure_wsl_networking(distro)?;
            (
                wsl_bootstrap_script(distro, project, executable, connection),
                terminal_title(project),
                project.to_string(),
            )
        }
    };

    spawn_windows_terminal(&title, &script)?;
    Ok(CodexLaunchResult {
        environment: config.environment,
        project_directory,
    })
}

pub fn validate(config: &CodexLauncherConfig) -> Result<(), String> {
    match config.environment {
        CodexLaunchEnvironment::Windows => {
            let project = required_value(
                config.windows_project_directory.as_deref(),
                "Windowsのプロジェクトディレクトリを選択してください",
            )?;
            validate_windows_project(project)
        }
        CodexLaunchEnvironment::Wsl => {
            let distro = required_value(
                config.wsl_distribution.as_deref(),
                "WSLディストリビューションを選択してください",
            )?;
            let project = required_value(
                config.wsl_project_directory.as_deref(),
                "WSLのプロジェクトディレクトリを入力してください",
            )?;
            let executable = required_value(
                Some(config.wsl_executable.as_str()),
                "WSL側のCodex実行ファイルを入力してください",
            )?;
            validate_shell_value(distro, "WSLディストリビューション")?;
            validate_wsl_project(project)?;
            validate_shell_value(executable, "WSL側のCodex実行ファイル")?;
            ensure_wsl_networking(distro)
        }
    }
}

fn validate_connection(connection: &CodexClientLaunchInfo) -> Result<(), String> {
    if !connection.broker_token_path.is_file() {
        return Err("Broker認証トークンが見つかりません。連携を再起動してください".to_string());
    }
    if connection.broker_port < 1024 {
        return Err("Brokerポートが不正です".to_string());
    }
    Ok(())
}

fn required_value<'a>(value: Option<&'a str>, message: &str) -> Result<&'a str, String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| message.to_string())
}

fn validate_windows_project(project: &str) -> Result<(), String> {
    validate_shell_value(project, "Windowsプロジェクトディレクトリ")?;
    let path = Path::new(project);
    if !path.is_absolute() {
        return Err("Windowsプロジェクトディレクトリは絶対パスで指定してください".to_string());
    }
    if !path.is_dir() {
        return Err(format!(
            "Windowsプロジェクトディレクトリが見つかりません: {project}"
        ));
    }
    Ok(())
}

fn validate_wsl_project(project: &str) -> Result<(), String> {
    validate_shell_value(project, "WSLプロジェクトディレクトリ")?;
    if !project.starts_with('/') {
        return Err("WSLプロジェクトディレクトリは / から始めてください".to_string());
    }
    Ok(())
}

fn validate_shell_value(value: &str, label: &str) -> Result<(), String> {
    if value.contains(['\r', '\n', '\0']) {
        return Err(format!("{label}に改行またはNUL文字は使用できません"));
    }
    Ok(())
}

fn windows_bootstrap_script(project: &str, connection: &CodexClientLaunchInfo) -> String {
    let project = powershell_quote(project);
    let token_path = powershell_quote(&display_path(&connection.broker_token_path));
    let executable = connection
        .windows_executable
        .as_deref()
        .ok_or_else(|| "Windows Codex executable is unavailable".to_string())
        .map(|path| powershell_quote(&display_path(path)));
    let executable = executable.expect("Windows launcher requires a Windows executable");
    let remote = powershell_quote(&format!("ws://127.0.0.1:{}", connection.broker_port));
    format!(
        "$ErrorActionPreference = 'Stop'; \
         Set-Location -LiteralPath {project}; \
         $env:{BROKER_TOKEN_ENV} = (Get-Content -LiteralPath {token_path} -Raw).Trim(); \
         try {{ & {executable} -C {project} --remote {remote} --remote-auth-token-env {BROKER_TOKEN_ENV} }} \
         finally {{ Remove-Item 'Env:{BROKER_TOKEN_ENV}' -ErrorAction SilentlyContinue }}"
    )
}

fn wsl_bootstrap_script(
    distro: &str,
    project: &str,
    executable: &str,
    connection: &CodexClientLaunchInfo,
) -> String {
    let distro = powershell_quote(distro);
    let project = powershell_quote(project);
    let executable = powershell_quote(executable);
    let token_path = powershell_quote(&display_path(&connection.broker_token_path));
    let remote = powershell_quote(&format!("ws://127.0.0.1:{}", connection.broker_port));
    let shell_script = powershell_quote(&format!(
        "\"$1\" -C \"$2\" --remote \"$3\" --remote-auth-token-env {BROKER_TOKEN_ENV}; \
         unset {BROKER_TOKEN_ENV}; \
         exec \"${{SHELL:-/bin/sh}}\" -l"
    ));
    format!(
        "$ErrorActionPreference = 'Stop'; \
         $previousWslEnv = $env:WSLENV; \
         $env:{BROKER_TOKEN_ENV} = (Get-Content -LiteralPath {token_path} -Raw).Trim(); \
         $wslEnvEntries = @($env:WSLENV -split ':' | Where-Object {{ $_ -and $_ -notmatch '^{BROKER_TOKEN_ENV}($|/)' }}); \
         $env:WSLENV = (($wslEnvEntries + '{BROKER_TOKEN_ENV}') -join ':'); \
         try {{ & 'wsl.exe' '-d' {distro} '--cd' {project} '--exec' 'sh' '-lc' {shell_script} 'keylink-codex' {executable} {project} {remote} }} \
         finally {{ \
           Remove-Item 'Env:{BROKER_TOKEN_ENV}' -ErrorAction SilentlyContinue; \
           if ($null -eq $previousWslEnv) {{ Remove-Item 'Env:WSLENV' -ErrorAction SilentlyContinue }} else {{ $env:WSLENV = $previousWslEnv }} \
         }}"
    )
}

fn spawn_windows_terminal(title: &str, script: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("wt.exe")
            .args(windows_terminal_args(title, script))
            .spawn()
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "Windows Terminalを起動できません。Windows Terminalがインストールされ、wt.exeが有効か確認してください: {error}"
                )
            })
    }
    #[cfg(not(windows))]
    {
        let _ = (title, script);
        Err("CodexランチャーはWindows版Keylink Studioでのみ利用できます".to_string())
    }
}

fn windows_terminal_args(title: &str, script: &str) -> Vec<OsString> {
    vec![
        "-w".into(),
        "0".into(),
        "new-tab".into(),
        "--title".into(),
        format!("Codex: {title}").into(),
        "--suppressApplicationTitle".into(),
        "powershell.exe".into(),
        "-NoLogo".into(),
        "-NoExit".into(),
        "-EncodedCommand".into(),
        encode_powershell_command(script).into(),
    ]
}

fn encode_powershell_command(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    BASE64_STANDARD.encode(bytes)
}

fn ensure_wsl_networking(distro: &str) -> Result<(), String> {
    let distributions = list_wsl_distributions()?;
    let selected = distributions
        .iter()
        .find(|candidate| candidate.name.eq_ignore_ascii_case(distro))
        .ok_or_else(|| format!("WSLディストリビューションが見つかりません: {distro}"))?;
    if selected.version == 1 || wsl_mirrored_networking_enabled() {
        return Ok(());
    }
    Err(
        "WSL2からWindows上のCodex Brokerへ安全に接続するには、%UserProfile%\\.wslconfig の [wsl2] で networkingMode=mirrored を有効にし、WSLを再起動してください"
            .to_string(),
    )
}

fn wsl_mirrored_networking_enabled() -> bool {
    let Some(profile) = env::var_os("USERPROFILE") else {
        return false;
    };
    let path = PathBuf::from(profile).join(".wslconfig");
    let Ok(contents) = fs::read_to_string(path) else {
        return false;
    };
    contents.lines().any(|line| {
        let normalized = line
            .split(['#', ';'])
            .next()
            .unwrap_or_default()
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect::<String>();
        normalized == "networkingmode=mirrored"
    })
}

fn parse_wsl_distributions(text: &str) -> Vec<WslDistribution> {
    let mut result = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('*').trim();
        let mut parts = line
            .split_whitespace()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let Some(version) = parts.last().and_then(|value| value.parse::<u8>().ok()) else {
            continue;
        };
        if version != 1 && version != 2 {
            continue;
        }
        parts.pop();
        if parts.len() < 2 {
            continue;
        }
        parts.pop();
        let name = parts.join(" ");
        if !name.is_empty() {
            result.push(WslDistribution { name, version });
        }
    }
    result
}

fn decode_windows_output(bytes: &[u8]) -> String {
    let looks_utf16_le =
        bytes.starts_with(&[0xff, 0xfe]) || (bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0);
    if !looks_utf16_le {
        return String::from_utf8_lossy(bytes).to_string();
    }
    let start = usize::from(bytes.starts_with(&[0xff, 0xfe])) * 2;
    let utf16 = bytes[start..]
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&utf16)
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn display_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        value.strip_prefix(r"\\?\").unwrap_or(&value).to_string()
    }
}

fn terminal_title(project: &str) -> String {
    project
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("Codex")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rawhid_host_core::CodexAppServerRuntime;

    fn connection() -> CodexClientLaunchInfo {
        CodexClientLaunchInfo {
            runtime: CodexAppServerRuntime::Windows,
            windows_executable: Some(PathBuf::from(r"C:\Tools\Codex's bin\codex.cmd")),
            broker_token_path: PathBuf::from(r"C:\Temp\Keylink's token\broker.token"),
            broker_port: 4501,
        }
    }

    #[test]
    fn parses_utf16_wsl_distribution_list() {
        let text = "  NAME                   STATE           VERSION\r\n* Ubuntu 24.04           Running         2\r\n  Legacy                 Stopped         1\r\n";
        let bytes = text
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();

        assert_eq!(
            parse_wsl_distributions(&decode_windows_output(&bytes)),
            vec![
                WslDistribution {
                    name: "Ubuntu 24.04".to_string(),
                    version: 2,
                },
                WslDistribution {
                    name: "Legacy".to_string(),
                    version: 1,
                }
            ]
        );
    }

    #[test]
    fn windows_script_quotes_paths_and_never_contains_token_value() {
        let script = windows_bootstrap_script(r"C:\Work\日本語's project", &connection());

        assert!(script.contains(r"'C:\Work\日本語''s project'"));
        assert!(script.contains("--remote 'ws://127.0.0.1:4501'"));
        assert!(script.contains("Get-Content -LiteralPath"));
        assert!(!script.contains("secret-token-value"));
    }

    #[test]
    fn terminal_uses_one_encoded_powershell_command() {
        let script = windows_bootstrap_script(r"C:\Work\日本語's project", &connection());
        let args = windows_terminal_args("日本語 project", &script);
        let args = args
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert_eq!(args.iter().filter(|value| *value == "new-tab").count(), 1);
        assert_eq!(
            args.iter()
                .filter(|value| *value == "-EncodedCommand")
                .count(),
            1
        );
        assert!(!args.last().unwrap().contains(';'));

        let decoded = BASE64_STANDARD.decode(args.last().unwrap()).unwrap();
        let utf16 = decoded
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        assert_eq!(String::from_utf16(&utf16).unwrap(), script);
    }

    #[test]
    fn wsl_script_preserves_wslenv_and_keeps_an_interactive_shell() {
        let script = wsl_bootstrap_script(
            "Ubuntu",
            "/home/onigiri/my project",
            "/home/onigiri/bin/codex",
            &connection(),
        );

        assert!(script.contains("$previousWslEnv = $env:WSLENV"));
        assert!(script.contains("KEYLINK_CODEX_BROKER_TOKEN"));
        assert!(script.contains("unset KEYLINK_CODEX_BROKER_TOKEN"));
        assert!(script.contains("exec \"${SHELL:-/bin/sh}\" -l"));
        assert!(script.contains("'/home/onigiri/my project'"));
    }

    #[test]
    fn rejects_non_absolute_wsl_project() {
        assert!(validate_wsl_project("home/onigiri/project").is_err());
    }
}
