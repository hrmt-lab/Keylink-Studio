//! Open a folder in Windows Explorer with smart window reuse.
//!
//! Behaviour (see `open_folder`):
//! - If the target folder is already open in an Explorer window, bring that
//!   window to the foreground.
//! - Else, when `prefer_tab` is set and an Explorer window already exists, try
//!   (best-effort) to open the folder in a new tab of that window.
//! - Else open the folder in a new window. This also launches Explorer when it
//!   is not running.
//!
//! Any failure in the COM / tab path falls back to a plain new window so the
//! folder always opens. The new-tab path is best-effort: Windows 11 exposes no
//! public API to open an Explorer tab, so it is driven by keystrokes and can
//! fail depending on focus, OS version or keyboard layout.

#[cfg(windows)]
pub fn open_folder(path: &str, prefer_tab: bool) -> Result<(), String> {
    windows_impl::open_folder(path, prefer_tab)
}

#[cfg(not(windows))]
pub fn open_folder(_path: &str, _prefer_tab: bool) -> Result<(), String> {
    Err("open_folder is only supported on Windows".to_string())
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::thread::sleep;
    use std::time::Duration;

    use windows::core::{w, Interface, PCWSTR, VARIANT};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
        KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_CONTROL, VK_RETURN,
    };
    use windows::Win32::UI::Shell::{IShellWindows, IWebBrowser2, ShellExecuteW, ShellWindows};
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOWNORMAL,
    };

    // 'T' and 'L' have no named VK_* constants in the Windows SDK.
    const VK_T: VIRTUAL_KEY = VIRTUAL_KEY(0x54);
    const VK_L: VIRTUAL_KEY = VIRTUAL_KEY(0x4C);

    pub fn open_folder(path: &str, prefer_tab: bool) -> Result<(), String> {
        // The COM reuse path either handles the request (front / new tab) or
        // reports that it did not, in which case we open a new window. Any COM
        // error is also treated as "not handled".
        let handled = unsafe { try_smart_open(path, prefer_tab) }.unwrap_or(false);
        if handled {
            Ok(())
        } else {
            open_new_window(path)
        }
    }

    /// Returns `Ok(true)` when the folder was brought to front or a new tab was
    /// requested; `Ok(false)` when no usable Explorer window was found.
    unsafe fn try_smart_open(path: &str, prefer_tab: bool) -> Result<bool, String> {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = enumerate(path, prefer_tab);
        CoUninitialize();
        result
    }

    unsafe fn enumerate(path: &str, prefer_tab: bool) -> Result<bool, String> {
        let target = normalize(path);
        let shell_windows: IShellWindows =
            CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let count = shell_windows.Count().map_err(|e| e.to_string())?;

        let mut first_explorer: Option<HWND> = None;
        for i in 0..count {
            let Ok(dispatch) = shell_windows.Item(&VARIANT::from(i)) else {
                continue;
            };
            let Ok(browser) = dispatch.cast::<IWebBrowser2>() else {
                continue;
            };
            let Ok(location) = browser.LocationURL() else {
                continue;
            };
            let location = location.to_string();
            // Empty LocationURL = not a filesystem folder window (e.g. legacy
            // browser control); skip it for both matching and tab reuse.
            if location.is_empty() {
                continue;
            }
            if first_explorer.is_none() {
                first_explorer = hwnd_of(&browser);
            }
            if let Some(folder) = url_to_path(&location) {
                if normalize(&folder) == target {
                    if let Some(hwnd) = hwnd_of(&browser) {
                        bring_to_front(hwnd);
                        return Ok(true);
                    }
                }
            }
        }

        // Folder is not currently open. Optionally reuse an existing window as a
        // new tab (best-effort); otherwise let the caller open a new window.
        if prefer_tab {
            if let Some(hwnd) = first_explorer {
                open_in_new_tab(hwnd, path);
                return Ok(true);
            }
        }
        Ok(false)
    }

    unsafe fn hwnd_of(browser: &IWebBrowser2) -> Option<HWND> {
        browser.HWND().ok().map(|h| HWND(h.0 as *mut c_void))
    }

    unsafe fn bring_to_front(hwnd: HWND) {
        // Only un-minimize when iconic; SW_RESTORE on a maximized window would
        // un-maximize it.
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
    }

    /// Best-effort: focus the window, open a new tab (Ctrl+T), focus the address
    /// bar (Ctrl+L), type the path and press Enter.
    unsafe fn open_in_new_tab(hwnd: HWND, path: &str) {
        bring_to_front(hwnd);
        sleep(Duration::from_millis(120));
        send_chord(VK_T);
        sleep(Duration::from_millis(150));
        send_chord(VK_L);
        sleep(Duration::from_millis(80));
        type_text(path);
        send_key(VK_RETURN);
    }

    fn open_new_window(path: &str) -> Result<(), String> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let result = unsafe {
            ShellExecuteW(
                HWND::default(),
                w!("open"),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW returns an HINSTANCE; a value > 32 means success.
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecuteW failed (code {})", result.0 as isize))
        }
    }

    // ── SendInput helpers ──────────────────────────────────────────────

    unsafe fn send_chord(key: VIRTUAL_KEY) {
        send(&[
            key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
            key_input(key, KEYBD_EVENT_FLAGS(0)),
            key_input(key, KEYEVENTF_KEYUP),
            key_input(VK_CONTROL, KEYEVENTF_KEYUP),
        ]);
    }

    unsafe fn send_key(key: VIRTUAL_KEY) {
        send(&[
            key_input(key, KEYBD_EVENT_FLAGS(0)),
            key_input(key, KEYEVENTF_KEYUP),
        ]);
    }

    unsafe fn type_text(text: &str) {
        for unit in text.encode_utf16() {
            send(&[
                unicode_input(unit, KEYBD_EVENT_FLAGS(0)),
                unicode_input(unit, KEYEVENTF_KEYUP),
            ]);
        }
    }

    unsafe fn send(inputs: &[INPUT]) {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }

    fn key_input(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn unicode_input(scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan,
                    dwFlags: KEYEVENTF_UNICODE | flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    // ── Path helpers ───────────────────────────────────────────────────

    /// Normalise a filesystem path for comparison: unify separators, drop a
    /// trailing separator and lower-case (Windows paths are case-insensitive).
    fn normalize(path: &str) -> String {
        path.replace('/', "\\")
            .trim_end_matches('\\')
            .to_lowercase()
    }

    /// Convert a `file:///C:/dir` URL (as returned by `LocationURL`) to a
    /// Windows path, percent-decoding escapes like `%20`.
    fn url_to_path(url: &str) -> Option<String> {
        let rest = url.strip_prefix("file:///")?;
        Some(percent_decode(rest).replace('/', "\\"))
    }

    fn percent_decode(input: &str) -> String {
        let bytes = input.as_bytes();
        let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let (Some(h), Some(l)) =
                    (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
                {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    fn hex_val(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
}
