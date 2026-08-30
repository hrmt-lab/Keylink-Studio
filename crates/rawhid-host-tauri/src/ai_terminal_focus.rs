//! Bring the dedicated Windows Terminal window of an AI session (Codex CLI /
//! Claude Code) to the foreground when its ScreenKey is pressed.
//!
//! Each session launched by Keylink Studio owns a Windows Terminal window
//! keyed by a unique `terminal_target_id` (`codex-<hex>` / `claude-<hex>`,
//! see `claude_launcher.rs` / `codex_launcher.rs`). The window is found by
//! title suffix on every press rather than cached by HWND: caching would risk
//! focusing an unrelated window after OS HWND reuse once the original window
//! closes. See `docs/screenkey-terminal-focus-design.md` for the full design.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Derive the expected window-title suffix from a `terminal_target_id`: the
/// first 8 characters of the segment following the final `-`, upper-cased.
/// Matches `AiDisplayTarget::label()` in `state.rs`. Returns `None` when the
/// id is empty or has no `-` (nothing to key a window title on).
fn expected_suffix(terminal_target_id: &str) -> Option<String> {
    if terminal_target_id.is_empty() || !terminal_target_id.contains('-') {
        return None;
    }
    let tail = terminal_target_id.rsplit('-').next().unwrap_or_default();
    if tail.is_empty() {
        return None;
    }
    Some(
        tail.chars()
            .take(8)
            .collect::<String>()
            .to_ascii_uppercase(),
    )
}

/// Whether a window title matches an expected suffix produced by
/// `expected_suffix`. Titles from before this feature (e.g. `Codex:
/// <project>`) carry no such suffix and never match.
fn title_matches_suffix(title: &str, suffix: &str) -> bool {
    !suffix.is_empty() && title.ends_with(suffix)
}

/// Spawn the search-and-focus sequence on a background thread and return
/// immediately. `focusing` is released (set back to `false`) once the thread
/// finishes, regardless of outcome, so the caller must have already claimed
/// it via `compare_exchange`.
pub fn spawn_focus(terminal_target_id: String, focusing: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let _guard = FocusGuard(focusing);
        #[cfg(windows)]
        windows_impl::focus(&terminal_target_id);
        #[cfg(not(windows))]
        let _ = terminal_target_id;
    });
}

struct FocusGuard(Arc<AtomicBool>);

impl Drop for FocusGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextLengthW, GetWindowTextW, IsIconic,
        IsWindowVisible, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };

    const TERMINAL_WINDOW_CLASS: &str = "CASCADIA_HOSTING_WINDOW_CLASS";

    struct FindData {
        suffix: String,
        matches: Vec<HWND>,
    }

    pub fn focus(terminal_target_id: &str) {
        let Some(suffix) = super::expected_suffix(terminal_target_id) else {
            return;
        };
        let mut data = FindData {
            suffix,
            matches: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut FindData as isize));
        }
        // Exactly one hit required: zero means the window is gone (or never
        // existed), more than one is an unexpected collision. Either way,
        // doing nothing is the safe choice.
        if data.matches.len() != 1 {
            return;
        }
        let hwnd = data.matches[0];
        unsafe {
            // Only un-minimize when iconic; SW_RESTORE on a maximized window
            // would un-maximize it.
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            if SetForegroundWindow(hwnd).as_bool() {
                return;
            }
        }
        // SetForegroundWindow can be denied by the foreground-lock rules;
        // `wt` is allowed to activate its own window even then. Slower
        // (~1.1s), so only tried after the fast path fails.
        let _ = std::process::Command::new("wt.exe")
            .args(["-w", terminal_target_id, "focus-tab"])
            .spawn();
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut FindData);

        // Skip hidden windows so they cannot inflate the hit count into a
        // false "multiple matches" and mask the real (visible) window.
        // Minimized windows keep WS_VISIBLE, so this does not interfere with
        // the restore step below.
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let mut class_buf = [0u16; 256];
        let class_len = GetClassNameW(hwnd, &mut class_buf);
        if class_len == 0 {
            return BOOL(1);
        }
        let class_name = OsString::from_wide(&class_buf[..class_len as usize])
            .to_string_lossy()
            .to_string();
        if class_name != TERMINAL_WINDOW_CLASS {
            return BOOL(1);
        }

        let title_len = GetWindowTextLengthW(hwnd);
        if title_len == 0 {
            return BOOL(1);
        }
        let mut title_buf = vec![0u16; title_len as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut title_buf);
        if copied == 0 {
            return BOOL(1);
        }
        title_buf.truncate(copied as usize);
        let title = OsString::from_wide(&title_buf)
            .to_string_lossy()
            .to_string();

        if super::title_matches_suffix(&title, &data.suffix) {
            data.matches.push(hwnd);
            // No need to keep enumerating once a collision is confirmed.
            if data.matches.len() >= 2 {
                return BOOL(0);
            }
        }
        BOOL(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_suffix_derives_upper_case_tail() {
        assert_eq!(
            expected_suffix("codex-0123456789abcdef0123456789abcdef"),
            Some("01234567".to_string())
        );
    }

    #[test]
    fn expected_suffix_mixed_case_input_is_upper_cased() {
        assert_eq!(
            expected_suffix("claude-BdAe75190123456789abcdef01234567"),
            Some("BDAE7519".to_string())
        );
    }

    #[test]
    fn expected_suffix_none_for_empty_string() {
        assert_eq!(expected_suffix(""), None);
    }

    #[test]
    fn expected_suffix_none_without_dash() {
        assert_eq!(
            expected_suffix("codex0123456789abcdef0123456789abcdef"),
            None
        );
    }

    #[test]
    fn expected_suffix_none_when_tail_after_dash_is_empty() {
        assert_eq!(expected_suffix("codex-"), None);
    }

    #[test]
    fn title_matches_suffix_when_title_ends_with_it() {
        assert!(title_matches_suffix(
            "Claude Code \u{b7} Keylink-Studio \u{b7} 6789ABCD",
            "6789ABCD"
        ));
    }

    #[test]
    fn title_matches_suffix_rejects_legacy_title_without_suffix() {
        assert!(!title_matches_suffix("Codex: Keylink-Studio", "6789ABCD"));
    }

    #[test]
    fn title_matches_suffix_is_case_sensitive() {
        // Expected suffixes are always upper-cased by `expected_suffix`; a
        // lower-cased title tail must not match.
        assert!(!title_matches_suffix(
            "Claude Code \u{b7} Keylink-Studio \u{b7} 6789abcd",
            "6789ABCD"
        ));
    }

    #[test]
    fn title_matches_suffix_rejects_empty_suffix() {
        assert!(!title_matches_suffix("anything", ""));
    }

    // This whole feature hinges on one contract: the window title produced
    // by `commands::terminal_display_name` (used to `--title` the launched
    // `wt.exe` tab) must end with what `expected_suffix` derives from the
    // same `terminal_target_id`. These tests pin that contract directly
    // against `terminal_display_name` so a future change to its format
    // fails a test here instead of silently breaking focus at runtime.
    use crate::commands::terminal_display_name;

    #[test]
    fn terminal_display_name_title_matches_its_own_expected_suffix_for_codex() {
        let id = "codex-0123456789abcdef0123456789abcdef";
        let title = terminal_display_name("Codex", "C:\\projects\\keylink-studio", id);
        assert!(title_matches_suffix(&title, &expected_suffix(id).unwrap()));
    }

    #[test]
    fn terminal_display_name_title_matches_its_own_expected_suffix_for_claude() {
        let id = "claude-fedcba9876543210fedcba9876543210";
        let title = terminal_display_name("Claude Code", "/home/user/keylink-studio", id);
        assert!(title_matches_suffix(&title, &expected_suffix(id).unwrap()));
    }

    #[test]
    fn terminal_display_name_title_matches_expected_suffix_with_non_ascii_project_name() {
        let id = "codex-aabbccdd00112233aabbccdd00112233";
        let title = terminal_display_name(
            "Codex",
            "C:\\projects\\キーボード開発\\日本語プロジェクト",
            id,
        );
        assert!(title_matches_suffix(&title, &expected_suffix(id).unwrap()));
    }
}
