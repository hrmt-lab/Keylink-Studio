//! Launch an application, reusing an already-running instance when possible.
//!
//! `focus_or_launch` brings an existing window of the target executable to the
//! foreground if one is found; otherwise it launches it. The "already running"
//! check matches by executable file name (case-insensitive), consistent with
//! the `exe` matcher used for layer rules: full paths are fragile for apps
//! installed under versioned / hashed directories (e.g. Autodesk Fusion's
//! webdeploy folder, auto-updating Electron apps).
//!
//! The match key is, in order of precedence:
//! 1. `match_exe` override (for launcher-style apps whose window-owning exe
//!    differs from the launch path, e.g. Fusion `FusionLauncher.exe` vs
//!    `Fusion360.exe`),
//! 2. for a `.lnk` path, the resolved shortcut target's file name,
//! 3. otherwise the path's file name.
//!
//! Launching goes through `ShellExecuteW`, so `.lnk` shortcuts and file
//! associations work, not just plain executables.

pub fn focus_or_launch(path: &str, match_exe: Option<&str>) -> Result<(), String> {
    #[cfg(windows)]
    {
        if unsafe { windows_impl::bring_running_to_front(path, match_exe) } {
            return Ok(());
        }
        windows_impl::launch(path)
    }
    #[cfg(not(windows))]
    {
        let _ = match_exe;
        std::process::Command::new(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows::core::{w, Interface, PCWSTR};
    use windows::Win32::Foundation::{BOOL, CloseHandle, HWND, LPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellExecuteW, ShellLink};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, SW_RESTORE, SW_SHOWNORMAL,
        WS_EX_TOOLWINDOW,
    };

    struct FindData {
        target: String,
        hwnd: Option<HWND>,
    }

    /// Returns true when a window of the target executable was found and brought
    /// to the foreground.
    pub unsafe fn bring_running_to_front(path: &str, match_exe: Option<&str>) -> bool {
        let target = focus_key(path, match_exe);
        if target.is_empty() {
            return false;
        }
        let mut data = FindData { target, hwnd: None };
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut FindData as isize));
        if let Some(hwnd) = data.hwnd {
            // Only un-minimize when iconic; SW_RESTORE on a maximized window
            // would un-maximize it. SetForegroundWindow keeps the current
            // (maximized / normal) state.
            if IsIconic(hwnd).as_bool() {
                let _ = ShowWindow(hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(hwnd);
            true
        } else {
            false
        }
    }

    /// Launch via the shell so `.lnk` shortcuts and file associations work.
    pub fn launch(path: &str) -> Result<(), String> {
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

    /// Resolve the match key (lower-cased exe file name) per the precedence in
    /// the module docs.
    fn focus_key(path: &str, match_exe: Option<&str>) -> String {
        if let Some(m) = match_exe {
            if !m.trim().is_empty() {
                return file_name_key(m);
            }
        }
        if path.trim().to_lowercase().ends_with(".lnk") {
            return resolve_lnk_target(path)
                .map(|t| file_name_key(&t))
                .unwrap_or_default();
        }
        file_name_key(path)
    }

    /// Resolve a `.lnk` shortcut to its target path via `IShellLink`.
    fn resolve_lnk_target(path: &str) -> Option<String> {
        unsafe {
            if CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_err() {
                return None;
            }
            let target = resolve_lnk_inner(path).ok().filter(|s| !s.is_empty());
            CoUninitialize();
            target
        }
    }

    unsafe fn resolve_lnk_inner(path: &str) -> windows::core::Result<String> {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let persist: IPersistFile = link.cast()?;
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        persist.Load(PCWSTR(wide.as_ptr()), STGM_READ)?;
        let mut buf = [0u16; 260]; // MAX_PATH
        link.GetPath(&mut buf, std::ptr::null_mut(), 0)?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(String::from_utf16_lossy(&buf[..end]))
    }

    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = &mut *(lparam.0 as *mut FindData);

        // Only consider real, user-facing top-level windows (same filters as
        // `get_running_apps`): visible, not a tool window, and titled.
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return BOOL(1);
        }
        if GetWindowTextLengthW(hwnd) == 0 {
            return BOOL(1);
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return BOOL(1);
        }

        if let Some(exe_path) = process_path(pid) {
            if file_name_key(&exe_path) == data.target {
                data.hwnd = Some(hwnd);
                return BOOL(0); // stop enumeration
            }
        }
        BOOL(1)
    }

    fn process_path(pid: u32) -> Option<String> {
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut len,
            )
        };
        unsafe {
            let _ = CloseHandle(handle);
        }
        if result.is_err() || len == 0 {
            return None;
        }
        buffer.truncate(len as usize);
        Some(OsString::from_wide(&buffer).to_string_lossy().to_string())
    }

    /// Comparison key = lower-cased executable file name (basename). Stable
    /// across install location / version-folder changes, unlike a full path.
    /// `std::path` treats both `\` and `/` as separators on Windows.
    fn file_name_key(path: &str) -> String {
        std::path::Path::new(path.trim())
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.trim().to_string())
            .to_lowercase()
    }
}
