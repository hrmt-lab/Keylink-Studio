use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ActiveApp {
    pub process_path: Option<PathBuf>,
    pub exe: Option<String>,
    pub title: Option<String>,
}

pub trait ActiveAppProvider {
    fn active_app(&self) -> Result<ActiveApp, ActiveAppError>;
}

#[derive(Debug, Default)]
pub struct SystemActiveAppProvider;

impl ActiveAppProvider for SystemActiveAppProvider {
    fn active_app(&self) -> Result<ActiveApp, ActiveAppError> {
        platform::active_app()
    }
}

#[derive(Debug, Error)]
pub enum ActiveAppError {
    #[error("no foreground window")]
    NoForegroundWindow,
    #[error("active app inspection is only implemented on Windows")]
    UnsupportedPlatform,
    #[error("Windows API error: {0}")]
    Windows(String),
}

#[cfg(windows)]
mod platform {
    use super::{ActiveApp, ActiveAppError};
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows::core::PWSTR;
    use windows::Win32::{
        Foundation::{CloseHandle, HWND},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
        },
    };

    pub fn active_app() -> Result<ActiveApp, ActiveAppError> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.0.is_null() {
            return Err(ActiveAppError::NoForegroundWindow);
        }

        let title = window_title(hwnd)?;
        let mut process_id = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut process_id));
        }
        if process_id == 0 {
            return Err(ActiveAppError::NoForegroundWindow);
        }
        let process_path = process_path(process_id);
        let exe = process_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string());

        Ok(ActiveApp {
            process_path,
            exe,
            title,
        })
    }

    fn window_title(hwnd: HWND) -> Result<Option<String>, ActiveAppError> {
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len == 0 {
            return Ok(None);
        }
        let mut buffer = vec![0u16; len as usize + 1];
        let copied = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        if copied == 0 {
            return Ok(None);
        }
        buffer.truncate(copied as usize);
        Ok(Some(
            OsString::from_wide(&buffer).to_string_lossy().to_string(),
        ))
    }

    fn process_path(process_id: u32) -> Option<PathBuf> {
        if process_id == 0 {
            return None;
        }
        let handle =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;

        let mut buffer = vec![0u16; 32768];
        let mut copied = buffer.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                handle,
                PROCESS_NAME_WIN32,
                PWSTR(buffer.as_mut_ptr()),
                &mut copied,
            )
        };
        unsafe {
            let _ = CloseHandle(handle);
        }
        if result.is_err() || copied == 0 {
            return None;
        }
        buffer.truncate(copied as usize);
        Some(PathBuf::from(OsString::from_wide(&buffer)))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{ActiveApp, ActiveAppError};

    pub fn active_app() -> Result<ActiveApp, ActiveAppError> {
        Err(ActiveAppError::UnsupportedPlatform)
    }
}
