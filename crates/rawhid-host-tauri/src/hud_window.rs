//! A thin wrapper around a Tauri `WebviewWindow` that is meant to stay
//! visually on top of everything while never taking OS input focus (a
//! "HUD" window).
//!
//! This is production code: `HudWindow` itself is kept independent of
//! `hud_probe` (the verification harness in `hud_probe.rs`) so one can be
//! kept and the other discarded without entangling them.
//!
//! ## Why raw Win32 calls instead of Tauri's `window.show()` / `hide()` /
//! `set_focus()`
//!
//! Tauri's high-level `show()` goes through the platform window toolkit's
//! own "make visible" path, which on Windows does not guarantee it will
//! avoid activating the window — whether it activates can depend on the
//! window's current style, its owner/parent relationship, and internal
//! WebView2 behavior on first paint. There is no documented, stable
//! contract that `window.show()` never activates. `SetWindowPos(...,
//! SWP_NOACTIVATE | SWP_SHOWWINDOW)` and `ShowWindow(..., SW_HIDE)` are the
//! actual Win32 primitives that make the "don't activate" behavior an
//! explicit, requested flag rather than an implementation detail we are
//! hoping holds. Do not "simplify" `show_at` / `hide` back to
//! `window.show()` / `window.hide()` — that reintroduces exactly the
//! failure mode this type exists to avoid.
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// A non-activating, always-on-top window.
///
/// Holds the `WebviewWindow` plus the raw HWND value (as a plain `isize`,
/// not `windows::Win32::Foundation::HWND`) so the type stays trivially
/// `Send`/`Sync` — `HWND` wraps a raw pointer and does not implement
/// `Send`/`Sync` itself, which would otherwise make it awkward to hand a
/// `HudWindow` to a background thread (as `hud_probe` does).
pub struct HudWindow {
    window: WebviewWindow,
    hwnd_raw: isize,
}

impl HudWindow {
    /// Creates the window hidden, non-activating, always-on-top, and
    /// without decorations/taskbar presence.
    ///
    /// `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` is applied to the extended
    /// window style **here**, immediately after the underlying HWND exists
    /// and strictly before the window is ever shown (it is created with
    /// `visible(false)` in the first place). This ordering matters: these
    /// styles govern how the window behaves when it transitions from hidden
    /// to visible, so if they were applied later — e.g. lazily on the first
    /// call to `show_at` — there is no guarantee the OS/WebView2 hasn't
    /// already made an activation decision based on the style the window
    /// had at creation time. Setting the bits up front, before any show, is
    /// the only sequencing that has been treated as safe here.
    pub fn create(app: &AppHandle, label: &str, url: WebviewUrl) -> Result<Self, String> {
        let window = WebviewWindowBuilder::new(app, label, url)
            .visible(false)
            .focused(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .decorations(false)
            .resizable(false)
            // Lets the HUD panel show a translucent surface (its own
            // background is painted at bg-surface/90 in Hud.tsx, with
            // hud/hud.css clearing html/body's opaque background from
            // index.css) instead of an opaque window rectangle. Requires
            // `decorations(false)` above, which this window already has.
            .transparent(true)
            // Without this, DWM draws its own drop shadow around the
            // undecorated window rectangle. That shadow has square corners,
            // so it stays visible outside the panel's `rounded-card` edges
            // and the window reads as square no matter how the panel itself
            // is rounded. The panel draws its own edge (`ring-1 ring-border`).
            .shadow(false)
            .build()
            .map_err(|err| format!("failed to create HUD window: {err}"))?;

        let hwnd_raw = apply_noactivate_style(&window)?;

        Ok(Self { window, hwnd_raw })
    }

    /// Shows the window at the given position/size without activating it.
    ///
    /// Uses `SetWindowPos(HWND_TOPMOST, ..., SWP_NOACTIVATE | SWP_SHOWWINDOW)`
    /// rather than Tauri's `window.show()` / `window.set_position()` — see
    /// the module doc for why.
    #[cfg(windows)]
    pub fn show_at(&self, x: i32, y: i32, w: i32, h: i32) {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        };

        unsafe {
            let _ = SetWindowPos(
                self.hwnd(),
                HWND_TOPMOST,
                x,
                y,
                w,
                h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
        }
    }

    #[cfg(not(windows))]
    pub fn show_at(&self, _x: i32, _y: i32, _w: i32, _h: i32) {
        // No-op outside Windows: this HUD is built entirely on Win32
        // z-order/activation primitives that have no equivalent here.
    }

    /// Hides the window via `ShowWindow(SW_HIDE)` rather than Tauri's
    /// `window.hide()` — see the module doc for why.
    #[cfg(windows)]
    pub fn hide(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_HIDE);
        }
    }

    #[cfg(not(windows))]
    pub fn hide(&self) {}

    /// The raw HWND value, for code that must hide this window from
    /// another thread later (see `hud_coordinator.rs`'s exit animation).
    ///
    /// `HWND` is a raw-pointer newtype and therefore not `Send`, which is
    /// why this type stores the handle as an `isize` in the first place;
    /// callers move the value and rebuild the handle on the far side.
    pub fn hwnd_raw(&self) -> isize {
        self.hwnd_raw
    }

    /// [`Self::hide`] for a window identified only by its raw handle
    /// value, so a thread that outlives the borrow can still hide it.
    #[cfg(windows)]
    pub fn hide_raw(hwnd_raw: isize) {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

        unsafe {
            let _ = ShowWindow(HWND(hwnd_raw as *mut core::ffi::c_void), SW_HIDE);
        }
    }

    #[cfg(not(windows))]
    pub fn hide_raw(_hwnd_raw: isize) {}

    /// Shows the window and forcibly activates it.
    ///
    /// **Never call this in production.** It exists solely so
    /// `hud_probe`'s foreground-change hook has a known-good positive case
    /// to confirm the measurement instrument itself is alive (see phase P5
    /// in `hud_probe.rs`) — without a deliberate positive case, a probe run
    /// that captures zero foreground-change events is indistinguishable
    /// from a broken/inactive hook, which would otherwise look like a
    /// false "PASS".
    #[cfg(windows)]
    pub fn show_activating_for_instrument_check(&self) {
        use windows::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOW};

        unsafe {
            let _ = ShowWindow(self.hwnd(), SW_SHOW);
            let _ = SetForegroundWindow(self.hwnd());
        }
    }

    #[cfg(not(windows))]
    pub fn show_activating_for_instrument_check(&self) {}

    /// The underlying Tauri window/webview handle (e.g. to call `.emit()`
    /// on it or query monitor geometry).
    pub fn window(&self) -> &WebviewWindow {
        &self.window
    }

    /// The window's raw HWND, reconstructed from the stored `isize` on
    /// every call.
    #[cfg(windows)]
    pub fn hwnd(&self) -> windows::Win32::Foundation::HWND {
        windows::Win32::Foundation::HWND(self.hwnd_raw as *mut core::ffi::c_void)
    }
}

/// Applies `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` to the window's extended
/// style and returns the raw HWND value (as `isize`) for later reuse.
///
/// Tauri's `WebviewWindow::hwnd()` returns a `windows::Win32::Foundation::
/// HWND` from whatever version of the `windows` crate Tauri itself was
/// built against (`0.61.x` at the time of writing), which is a distinct
/// Rust type from this crate's own pinned `windows 0.58` — the two are not
/// interchangeable even though they have the same `#[repr(transparent)]
/// (pub *mut c_void)` layout. Converting through the raw pointer value
/// (`.0`) sidesteps the version mismatch entirely.
#[cfg(windows)]
fn apply_noactivate_style(window: &WebviewWindow) -> Result<isize, String> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };

    let tauri_hwnd = window
        .hwnd()
        .map_err(|err| format!("failed to read HUD window HWND: {err}"))?;
    let hwnd_raw = tauri_hwnd.0 as isize;
    let hwnd = HWND(hwnd_raw as *mut core::ffi::c_void);

    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let new_style = ex_style | (WS_EX_NOACTIVATE.0 as isize) | (WS_EX_TOOLWINDOW.0 as isize);
        if new_style != ex_style {
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, new_style);
        }
    }

    Ok(hwnd_raw)
}

#[cfg(not(windows))]
fn apply_noactivate_style(_window: &WebviewWindow) -> Result<isize, String> {
    Ok(0)
}
