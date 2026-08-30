use std::{ffi::OsString, path::Path, process::Command};

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use rawhid_host_core::{ClaudeLauncherConfig, ClaudePluginArtifacts};
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ClaudeLaunchResult {
    pub project_directory: String,
    pub plugin_directory: String,
    pub terminal_target_id: String,
    pub display_name: String,
}

pub fn launch(
    config: &ClaudeLauncherConfig,
    artifacts: &ClaudePluginArtifacts,
    helper_executable: &Path,
    terminal_target_id: &str,
    display_name: &str,
) -> Result<ClaudeLaunchResult, String> {
    validate(config)?;
    if !helper_executable.is_file() {
        return Err(format!(
            "Claude hook Helperが見つかりません: {}",
            helper_executable.display()
        ));
    }
    let project = config.project_directory.as_deref().expect("validated");
    let executable = config.executable_path.as_deref().unwrap_or("claude");
    let script = format!(
        "& {} -ClaudeExecutable {} -ProjectDirectory {} -PluginRoot {} -ObserverPath {} -HelperExecutable {}",
        quote_path(&artifacts.wrapper_path),
        quote_string(executable),
        quote_string(project),
        quote_path(&artifacts.plugin_root),
        quote_path(&artifacts.observer_path),
        quote_path(helper_executable),
    );
    spawn_windows_terminal(terminal_target_id, display_name, &script)?;
    Ok(ClaudeLaunchResult {
        project_directory: project.to_string(),
        plugin_directory: artifacts.plugin_root.display().to_string(),
        terminal_target_id: terminal_target_id.to_string(),
        display_name: display_name.to_string(),
    })
}

pub fn validate(config: &ClaudeLauncherConfig) -> Result<(), String> {
    let project = config
        .project_directory
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "Claude Codeのプロジェクトディレクトリを選択してください".to_string())?;
    if project.contains(['\r', '\n', '\0']) {
        return Err(
            "Claude Codeのプロジェクトディレクトリに改行またはNUL文字は使用できません".to_string(),
        );
    }
    let project_path = Path::new(project);
    if !project_path.is_absolute() || !project_path.is_dir() {
        return Err(format!(
            "Claude Codeのプロジェクトディレクトリが見つかりません: {project}"
        ));
    }
    if let Some(executable) = config.executable_path.as_deref() {
        if executable.contains(['\r', '\n', '\0']) {
            return Err("Claude Code実行ファイルに改行またはNUL文字は使用できません".to_string());
        }
        let path = Path::new(executable);
        if path.is_absolute() && !path.is_file() {
            return Err(format!(
                "Claude Code実行ファイルが見つかりません: {executable}"
            ));
        }
    }
    Ok(())
}

fn spawn_windows_terminal(
    terminal_target_id: &str,
    title: &str,
    script: &str,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        Command::new("wt.exe")
            .args(windows_terminal_args(terminal_target_id, title, script))
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("Windows TerminalでClaude Codeを起動できません: {error}"))
    }
    #[cfg(not(windows))]
    {
        let _ = (terminal_target_id, title, script);
        Err("Claude CodeランチャーはWindows版Keylink Studioでのみ利用できます".to_string())
    }
}

fn windows_terminal_args(terminal_target_id: &str, title: &str, script: &str) -> Vec<OsString> {
    vec![
        "-w".into(),
        terminal_target_id.into(),
        "new-tab".into(),
        "--title".into(),
        title.into(),
        "--suppressApplicationTitle".into(),
        "powershell.exe".into(),
        "-NoLogo".into(),
        "-NoExit".into(),
        "-EncodedCommand".into(),
        encode_powershell_command(script).into(),
    ]
}

fn encode_powershell_command(script: &str) -> String {
    BASE64_STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    )
}

fn quote_path(value: &Path) -> String {
    quote_string(&value.display().to_string())
}

fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_uses_one_encoded_command() {
        let args = windows_terminal_args(
            "claude-0123456789abcdef0123456789abcdef",
            "project",
            "& 'C:\\a b\\wrapper.ps1' -ClaudeExecutable 'claude'",
        );
        assert_eq!(
            args.iter()
                .filter(|value| *value == "-EncodedCommand")
                .count(),
            1
        );
        let decoded = BASE64_STANDARD
            .decode(args.last().unwrap().to_string_lossy().as_bytes())
            .unwrap();
        let units = decoded
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        assert!(String::from_utf16(&units).unwrap().contains("wrapper.ps1"));
    }
}
