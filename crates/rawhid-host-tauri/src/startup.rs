//! "Launch at login" via the per-user Windows Run registry key.

#[cfg(windows)]
const RUN_VALUE_NAME: &str = "RawHID Host";

pub fn is_launch_at_login() -> bool {
    #[cfg(windows)]
    {
        windows_impl::is_enabled()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn set_launch_at_login(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_impl::set_enabled(enabled)
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("launch at login is only supported on Windows".to_string())
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
        HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
    };

    use super::RUN_VALUE_NAME;

    const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open(access: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Option<HKEY> {
        let mut hkey = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(wide(RUN_KEY_PATH).as_ptr()),
                0,
                access,
                &mut hkey,
            )
        };
        (status == ERROR_SUCCESS).then_some(hkey)
    }

    pub fn is_enabled() -> bool {
        let Some(hkey) = open(KEY_READ) else {
            return false;
        };
        let name = wide(RUN_VALUE_NAME);
        let status =
            unsafe { RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None) };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        status == ERROR_SUCCESS
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let hkey = open(KEY_READ | KEY_WRITE).ok_or("failed to open Run registry key")?;
        let name = wide(RUN_VALUE_NAME);
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let command = format!("\"{}\"", exe.to_string_lossy());
            let data = wide(&command);
            let bytes =
                unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 2) };
            let status =
                unsafe { RegSetValueExW(hkey, PCWSTR(name.as_ptr()), 0, REG_SZ, Some(bytes)) };
            if status == ERROR_SUCCESS {
                Ok(())
            } else {
                Err("failed to write Run registry value".to_string())
            }
        } else {
            let status = unsafe { RegDeleteValueW(hkey, PCWSTR(name.as_ptr())) };
            // Deleting a value that does not exist is treated as success.
            if status == ERROR_SUCCESS || status == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND
            {
                Ok(())
            } else {
                Err("failed to remove Run registry value".to_string())
            }
        };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        result
    }
}
