//! `--hud-focus-probe` verification harness.
//!
//! Measures — by instrumentation, not by eyeballing it — whether
//! `HudWindow` (see `hud_window.rs`) ever steals OS foreground/focus while
//! it is shown, hidden, updated, or clicked on top of WebView2. This module
//! is throwaway verification code and is deliberately kept independent of
//! `hud_window.rs` so either can be discarded without unwinding the other.
//!
//! Entry point: [`run`] (invoked from `run_hud_focus_probe()` in `lib.rs`,
//! which is reached via `--hud-focus-probe` on the command line — see
//! `main.rs`). Writes `hud-focus-probe-<yyyyMMdd-HHmmss>.log` to the current
//! directory; also `println!`s the same content in debug builds, since a
//! release build has no console (`windows_subsystem = "windows"`).
//!
//! The probe page (`ui/public/hud-probe.html`) is a bare static HTML file
//! outside Vite's bundle rather than a normal routed page in `ui/src/`,
//! *because this whole harness is throwaway*: it exists only to be deleted
//! once the HUD's non-activating behavior is confirmed on real hardware. A
//! production HUD page should instead be a real bundled route so it can
//! `import` `@tauri-apps/api` normally — which needs neither
//! `withGlobalTauri` (an app-wide config flag that would also expose
//! `window.__TAURI__` on the production `main` window) nor the content
//! updates in this probe having to go through `WebviewWindow::eval` (see
//! `set_hud_text` below) instead of a proper Tauri event.
//!
//! Two real-hardware runs each found a hole in the measurement itself
//! (not in the HUD) and both are closed here:
//! - Run 1 (debug build, dev server not running): the HUD silently showed
//!   a Chromium `ERR_CONNECTION_REFUSED` page instead of `hud-probe.html`,
//!   so P3's DOM updates measured nothing while still reporting a clean
//!   "0 events". `check_dev_server_reachable` now VOIDs the run before any
//!   phase executes if that page would be unreachable.
//! - Run 2: the `SetWinEventHook(..., WINEVENT_OUTOFCONTEXT)` hook
//!   (`ForegroundProbe`) silently dropped two guaranteed real foreground
//!   transitions. `ForegroundSampler` (a second, independent 20ms-polling
//!   instrument) was added specifically because of this; both instruments
//!   must be silent for a phase to pass — see the judgement loop in
//!   `build_log` for why neither one alone is a sufficient gate.
//! - Run 3 (`--release`): `check_dev_server_reachable` VOIDed every single
//!   release run, because it gated on `dev_url.is_some()` — a static
//!   config value that is `Some` regardless of build profile — instead of
//!   `tauri::is_dev()`. Combined with a release build having no console
//!   (`main.rs`'s `windows_subsystem = "windows"`), this made
//!   `--release -- --hud-focus-probe` look like it silently did nothing.
//!   Both are fixed: the dev-server check now asks `tauri::is_dev()`
//!   directly, and every exit path shows a `MessageBoxW` with the
//!   headline result and the log's absolute path.
//! - Runs 4 and 5 found a flaw in the *experiment design*, not the HUD:
//!   both real runs showed the hook's very last firing of the whole run
//!   was P4's click (t=35298ms and t=32995ms respectively), completely
//!   silent afterward. That is not random drop-out — clicking the HUD
//!   reproducibly leaves the hook unable to observe further transitions.
//!   The old phase order (`P0b, P1-P3, P4, P5`) put the *only* postcheck*
//!   on the far side of that click, so it was structurally guaranteed to
//!   VOID. Phases are now ordered `P0b, P1-P3, P3b (postcheck), P4
//!   (click, last), P4b (informational only)` — the checks that must be
//!   trusted (P1-P3's zeros) are bracketed by *two* working checks (P0b
//!   and P3b), and the click that breaks the hook happens only after
//!   both are already done. Run 5 also showed the P0b/P3b retries were
//!   pointless once the HUD was already foreground: re-activating an
//!   already-foreground window creates no transition for either
//!   instrument to see. `run_instrument_check_with_retries` now restores
//!   a different foreground window between attempts so each retry is an
//!   actual chance to observe something.

#[cfg(windows)]
pub use windows_impl::run;

#[cfg(not(windows))]
pub fn run(_app: tauri::AppHandle) {
    // There is no Win32 foreground/focus surface to measure outside
    // Windows, and `HudWindow`'s Win32-backed methods are no-ops there
    // too. Nothing meaningful for this probe to do.
}

#[cfg(windows)]
mod windows_impl {
    use std::{
        cell::RefCell,
        fmt::Write as _,
        fs,
        io::{Read, Write as _},
        net::{TcpStream, ToSocketAddrs},
        path::{Path, PathBuf},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            mpsc, Arc, Mutex,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    use tauri::{AppHandle, WebviewUrl};
    use windows::{
        core::HSTRING,
        Win32::{
            Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
            UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
            UI::Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_LBUTTON,
            },
            UI::WindowsAndMessaging::{
                DispatchMessageW, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetMessageW,
                GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, MessageBoxW,
                PostThreadMessageW, SetForegroundWindow, TranslateMessage, EVENT_SYSTEM_FOREGROUND,
                GUITHREADINFO, MB_ICONINFORMATION, MB_OK, MSG, WINEVENT_OUTOFCONTEXT, WM_QUIT,
            },
        },
    };

    use crate::hud_window::HudWindow;

    const HUD_WIDTH: i32 = 420;
    const HUD_HEIGHT: i32 = 260;
    const MONITOR_MARGIN: i32 = 24;
    /// How long the foreground/caret state must stay unchanged before
    /// `wait_for_quiescence` considers it settled.
    const QUIESCENCE_MIN_STABLE: Duration = Duration::from_millis(500);
    /// Ceiling on how long `wait_for_quiescence` will wait before giving up
    /// and reporting a timeout.
    const QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(5);

    /// Writes `text` into the probe page's `#hud-probe-text` element by
    /// injecting JS directly into the webview via `WebviewWindow::eval`,
    /// **not** a Tauri event (`.emit()`).
    ///
    /// `.emit()` is IPC: the frontend has to `listen()` for it, which on a
    /// bare (non-bundled) HTML page only works via `window.__TAURI__`,
    /// which in turn requires the app-wide `withGlobalTauri` config flag —
    /// and that flag would have exposed `window.__TAURI__` on the
    /// production `main` window too, for the sake of a throwaway probe.
    /// `eval()` sidesteps all of that: it's a direct Rust -> webview JS
    /// injection with no IPC/capability surface involved, so it needs
    /// neither `withGlobalTauri` nor a `hud-probe` capability file. It is
    /// also, arguably, the more honest measurement for P3: no extra IPC
    /// hop between "Rust decides to update the HUD" and "the DOM changes."
    fn set_hud_text(hud: &HudWindow, text: &str) {
        // `serde_json::to_string` on a `&str` produces a quoted, escaped
        // JSON string literal, which is also a valid JS string literal —
        // safe to splice directly into the injected script regardless of
        // what `text` contains (quotes, backslashes, newlines, ...).
        let js_literal = serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string());
        let js = format!(
            "var el = document.getElementById('hud-probe-text'); \
             if (el) {{ el.textContent = {js_literal}; }}"
        );
        let _ = hud.window().eval(js);
    }

    /// Polls for a left-click landing inside the HUD's rect during
    /// `duration`, updating the on-HUD instruction text with a live
    /// countdown. Returns whether a qualifying click was detected.
    ///
    /// No `WH_MOUSE`/hook is needed: every 20ms this checks
    /// `GetAsyncKeyState(VK_LBUTTON)` for a down-edge (not down last poll,
    /// down now) and, only on that edge, `GetCursorPos()` against the HUD
    /// rect. A click detected this way is unambiguous evidence a human
    /// clicked *this* window, closing the gap the previous version of this
    /// probe had: without an explicit detector, "clicked but nothing
    /// happened" and "never clicked at all" both looked like a PASS.
    fn wait_for_hud_click(
        hud: &HudWindow,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        duration: Duration,
    ) -> bool {
        let deadline = Instant::now() + duration;
        let mut was_down = false;
        let mut detected = false;
        // Forces the first countdown text to be written immediately below,
        // instead of waiting a full second for the first update.
        let mut last_text_update = Instant::now() - Duration::from_secs(1);

        while Instant::now() < deadline {
            if !detected && last_text_update.elapsed() >= Duration::from_secs(1) {
                let remaining = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
                set_hud_text(
                    hud,
                    &format!("ここをクリックしてください\n(残り {remaining}秒)"),
                );
                last_text_update = Instant::now();
            }

            let state = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) };
            // High bit of the return value means the button is down right
            // now (per `GetAsyncKeyState` docs); the low bit tracks
            // "pressed since the last call", which is not what we want
            // here since we're polling on our own schedule, not driven by
            // a message loop.
            let is_down = (state as u16 & 0x8000) != 0;
            if is_down && !was_down && !detected {
                let mut point = POINT::default();
                let inside = unsafe {
                    GetCursorPos(&mut point).is_ok()
                        && point.x >= x
                        && point.x < x + w
                        && point.y >= y
                        && point.y < y + h
                };
                if inside {
                    detected = true;
                    set_hud_text(hud, "検出しました");
                }
            }
            was_down = is_down;

            thread::sleep(Duration::from_millis(20));
        }

        detected
    }

    /// Last-resort, human-visible fallback: gives the user up to
    /// `duration` to manually return foreground focus to `target_hwnd`
    /// (typically their editor) after a P0b/P3b instrument check, showing
    /// a countdown on the HUD in the meantime. Returns immediately (no
    /// flash) if the foreground is already `target_hwnd`.
    ///
    /// `run_instrument_check_with_retries` already tries to restore
    /// `target_hwnd` programmatically via `SetForegroundWindow` between
    /// attempts and once more before returning, which is expected to
    /// succeed in the common case (the probe's own process was the most
    /// recent one to change the foreground, which Windows' foreground-lock
    /// rules generally still permit). This function exists for the
    /// remaining case where that programmatic attempt is denied anyway —
    /// rather than silently leaving the HUD in the foreground going into
    /// the next phase, it asks the human to click back, with an on-HUD
    /// countdown, before proceeding. Returns whether `target_hwnd` was
    /// confirmed foreground again before the deadline; this is
    /// informational only — P0b/P3b's pass/fail comes from the
    /// instrument-health check in `build_log`, not from this.
    fn wait_for_foreground_return(
        hud: &HudWindow,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        target_hwnd: isize,
        duration: Duration,
    ) -> bool {
        if focus_snapshot().foreground_hwnd == target_hwnd {
            return true;
        }

        hud.show_at(x, y, w, h);
        let deadline = Instant::now() + duration;
        let mut returned = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now()).as_secs() + 1;
            set_hud_text(
                hud,
                &format!("エディタをクリックして前面に戻してください\n(残り {remaining}秒)"),
            );
            if focus_snapshot().foreground_hwnd == target_hwnd {
                returned = true;
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        hud.hide();
        returned
    }

    /// Reconstructs a `windows::Win32::Foundation::HWND` from the raw
    /// value `FocusSnapshot`/`ForegroundEvent` store it as (see
    /// `hud_window.rs`'s `apply_noactivate_style` for why HWNDs are kept
    /// as plain `isize` everywhere in this module rather than as `HWND`
    /// values themselves — the short version: `HWND` wraps a raw pointer
    /// and is not `Send`).
    fn hwnd_from_isize(raw: isize) -> HWND {
        HWND(raw as *mut core::ffi::c_void)
    }

    /// One attempt of the P0b/P3b instrument self-check: how many hook and
    /// sampler events landed inside that single attempt's activation
    /// window, or that the attempt never actually ran (`skipped`).
    struct InstrumentCheckAttempt {
        attempt: u32,
        hook_events: usize,
        sampler_events: usize,
        /// `true` when this attempt was abandoned before activating the
        /// HUD because a different-from-HUD foreground window could not
        /// be restored first (see `run_instrument_check_with_retries`).
        /// A skipped attempt's `hook_events`/`sampler_events` are both 0
        /// but must NOT be read as "the instrument stayed silent" — no
        /// activation happened at all, so there was nothing for either
        /// instrument to possibly observe.
        skipped: bool,
    }

    /// Outcome of `run_instrument_check_with_retries`: every attempt's
    /// counts, plus whether — and on which attempt — each instrument was
    /// ever confirmed alive.
    struct InstrumentCheckResult {
        attempts: Vec<InstrumentCheckAttempt>,
        hook_alive: bool,
        hook_first_attempt: Option<u32>,
        sampler_alive: bool,
        sampler_first_attempt: Option<u32>,
    }

    /// Runs the P0b/P3b instrument self-check activation
    /// (`show_activating_for_instrument_check()` → 1s → `hide()`) up to 3
    /// times, stopping as soon as both instruments have recorded at least
    /// one event across the attempts so far.
    ///
    /// A single-attempt check is too strict for a hook that is known to
    /// drop individual deliveries (see `ForegroundSampler`'s doc comment):
    /// a real run VOIDed with "hook fired at P0b but not at P5" even
    /// though the hook was demonstrably alive — it caught a real P4 click
    /// activation, just not the single postcheck activation. The fix is
    /// not to loosen what counts as "alive" (that would blunt the very
    /// thing this hook drop-out was supposed to catch); it is to give the
    /// check itself more chances to observe a real event, the same way a
    /// flaky sensor gets re-read rather than its threshold lowered.
    ///
    /// `restore_target_hwnd` is the foreground window this check should
    /// activate *away from* on every attempt. This matters because the
    /// first version of this function retried blindly and mostly retried
    /// nothing: after attempt 1 leaves the HUD in the foreground,
    /// `show_activating_for_instrument_check()` on attempt 2 re-activates
    /// a window that is *already* foreground — no transition occurs, so
    /// neither instrument has anything to detect, and the "retry" is a
    /// guaranteed no-op (`attempt 2: hook=0 sampler=0`, `attempt 3: hook=0
    /// sampler=0` on a real run). Before every attempt but the first, this
    /// now confirms the foreground is back on `restore_target_hwnd` —
    /// hiding the HUD and calling `SetForegroundWindow` if it isn't — so
    /// each attempt is a genuine, observable foreground *change*. If that
    /// restoration itself cannot be confirmed, the attempt is recorded as
    /// `skipped` (not as "the instrument didn't fire" — nothing was
    /// measurable) and retrying stops, since a HUD-already-foreground
    /// situation would just repeat.
    fn run_instrument_check_with_retries(
        hud: &HudWindow,
        start: Instant,
        hook: &ForegroundProbe,
        sampler: &ForegroundSampler,
        phase_id: &str,
        restore_target_hwnd: isize,
    ) -> InstrumentCheckResult {
        let mut attempts = Vec::new();
        let mut hook_alive = false;
        let mut hook_first_attempt = None;
        let mut sampler_alive = false;
        let mut sampler_first_attempt = None;

        for attempt in 1..=3u32 {
            if hook_alive && sampler_alive {
                break;
            }

            if focus_snapshot().foreground_hwnd != restore_target_hwnd {
                hud.hide();
                unsafe {
                    let _ = SetForegroundWindow(hwnd_from_isize(restore_target_hwnd));
                }
                // `SetForegroundWindow` is not guaranteed synchronous with
                // the OS's notion of "current foreground"; give it a beat
                // before re-checking.
                thread::sleep(Duration::from_millis(100));

                if focus_snapshot().foreground_hwnd != restore_target_hwnd {
                    // Could not get a different-from-HUD window back into
                    // the foreground (Windows' foreground-lock rules can
                    // deny this even to the process that just held
                    // foreground). A further activation here would just
                    // repeat "HUD activated over HUD" — abandon this
                    // attempt and stop, rather than burn the remaining
                    // budget on attempts that cannot possibly detect
                    // anything.
                    attempts.push(InstrumentCheckAttempt {
                        attempt,
                        hook_events: 0,
                        sampler_events: 0,
                        skipped: true,
                    });
                    break;
                }
            }

            let window_start = start.elapsed();
            hud.show_activating_for_instrument_check();
            thread::sleep(Duration::from_secs(1));
            hud.hide();
            let window_end = start.elapsed();

            let hook_events = hook
                .events_between(window_start, window_end, phase_id)
                .len();
            let sampler_events = sampler
                .events_between(window_start, window_end, phase_id)
                .len();

            if hook_events > 0 && !hook_alive {
                hook_alive = true;
                hook_first_attempt = Some(attempt);
            }
            if sampler_events > 0 && !sampler_alive {
                sampler_alive = true;
                sampler_first_attempt = Some(attempt);
            }

            attempts.push(InstrumentCheckAttempt {
                attempt,
                hook_events,
                sampler_events,
                skipped: false,
            });
        }

        // Leave the foreground back on the original window regardless of
        // how the loop ended — whatever phase runs next (P1 after P0b,
        // P4 after P3b) needs a non-HUD foreground to start from, and P4
        // specifically needs the editor foregrounded the moment it begins
        // clicking-detection.
        hud.hide();
        if focus_snapshot().foreground_hwnd != restore_target_hwnd {
            unsafe {
                let _ = SetForegroundWindow(hwnd_from_isize(restore_target_hwnd));
            }
        }

        InstrumentCheckResult {
            attempts,
            hook_alive,
            hook_first_attempt,
            sampler_alive,
            sampler_first_attempt,
        }
    }

    /// Outcome of a single `wait_for_quiescence` call: `Some(duration)` is
    /// how long the wait actually took once things settled; `None` means it
    /// timed out without ever seeing a stable state.
    struct QuiescenceOutcome {
        settled_after: Option<Duration>,
    }

    /// Blocks until the foreground has returned to `baseline_hwnd`, the
    /// caret has been recreated there, and neither instrument has recorded
    /// a new event for `min_stable`, or until `timeout` elapses.
    ///
    /// P0b/P3b deliberately move the foreground *twice* as part of the
    /// instrument self-check: to the HUD, then back to whatever was
    /// foreground before. That "back" transition is real cleanup work, not
    /// noise, but it still has to land somewhere in the event log — and
    /// without this wait, it reliably lands in the *next* phase instead of
    /// the one that caused it. Two real runs proved this is not a
    /// theoretical race:
    ///   - P0b's hook caught the return-to-editor transition at t=3144ms.
    ///     The sampler polls every 20ms, so it caught the very same
    ///     transition 18ms later, at t=3162ms — by which point `run_phase`
    ///     had already moved on to P1. That single sampler event was
    ///     counted as "P1 sampler=1" and failed KO-1 outright, even though
    ///     P1 itself never touched the foreground. Across every run so far,
    ///     P1-P3's own event logs show HUD activity exactly zero times —
    ///     every "FAIL" traced back to this leak, never a real activation.
    ///   - The identical pattern repeated at P3b -> P4: the hook caught the
    ///     HUD-then-back transitions at t=28641ms and t=29645ms; the
    ///     sampler caught the "back" half 19ms later, at t=29664ms, by
    ///     which point P4 had already started. P4's `before` snapshot was
    ///     also taken with `gui_caret_hwnd: 0` — the editor's caret hadn't
    ///     been recreated yet either, which is why this also checks
    ///     `gui_caret_hwnd != 0` and not just the foreground HWND.
    ///
    /// Calling this at the tail of P0b/P3b's `run_phase` body — inside
    /// their own time window — means any leftover transition it catches is
    /// attributed to the phase that actually caused it, not the next one.
    fn wait_for_quiescence(
        hook: &ForegroundProbe,
        sampler: &ForegroundSampler,
        start: Instant,
        phase_id: &str,
        baseline_hwnd: isize,
        min_stable: Duration,
        timeout: Duration,
    ) -> QuiescenceOutcome {
        let wait_start = Instant::now();
        let deadline = wait_start + timeout;
        let mut stable_since: Option<Instant> = None;
        let mut last_event_count: Option<usize> = None;

        loop {
            let now = Instant::now();
            if now >= deadline {
                return QuiescenceOutcome {
                    settled_after: None,
                };
            }

            let elapsed = start.elapsed();
            let event_count = hook.events_between(Duration::ZERO, elapsed, phase_id).len()
                + sampler
                    .events_between(Duration::ZERO, elapsed, phase_id)
                    .len();
            let snapshot = focus_snapshot();
            let foreground_settled = snapshot.foreground_hwnd == baseline_hwnd;
            let caret_ready = snapshot.gui_caret_hwnd != 0;
            let events_unchanged = last_event_count == Some(event_count);
            last_event_count = Some(event_count);

            if foreground_settled && caret_ready && events_unchanged {
                let since = *stable_since.get_or_insert(now);
                if now.duration_since(since) >= min_stable {
                    return QuiescenceOutcome {
                        settled_after: Some(now.duration_since(wait_start)),
                    };
                }
            } else {
                stable_since = None;
            }

            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Outcome of `fetch_hud_probe_html`'s raw HTTP GET of `/hud-probe.html`.
    enum ContentCheck {
        /// Got a 200 response containing `id="hud-probe-text"`: the real
        /// page is being served.
        Confirmed,
        /// Got a response, but it did not contain the marker — the wrong
        /// page (most likely a Vite/Chromium error page) is being served.
        /// This is real, actionable evidence, unlike `Inconclusive` below.
        MissingMarker,
        /// The minimal hand-rolled HTTP client here could not complete the
        /// check (connect/write/read/parse failure). This says nothing
        /// about whether the real page loads — TCP reachability was
        /// already confirmed by the caller — so it must never VOID the
        /// run by itself; it is surfaced as a warning only.
        Inconclusive(String),
    }

    /// Performs a bare-bones HTTP/1.1 GET for `/hud-probe.html` over a raw
    /// `TcpStream` and checks whether the response body contains
    /// `id="hud-probe-text"`.
    ///
    /// No `reqwest`: it is not a direct dependency of this crate (only
    /// `rawhid-host-core` depends on it), and pulling it in just for one
    /// diagnostic GET was avoided per instruction. This is deliberately
    /// minimal — one request, `Connection: close`, no redirects, no
    /// chunked-encoding handling — which is exactly why its failures are
    /// treated as `Inconclusive` rather than authoritative: a real
    /// (browser-grade) HTTP client could succeed where this one gives up.
    fn fetch_hud_probe_html(host: &str, port: u16) -> ContentCheck {
        let addr = format!("{host}:{port}");
        let socket_addr = match addr.to_socket_addrs().ok().and_then(|mut it| it.next()) {
            Some(addr) => addr,
            None => return ContentCheck::Inconclusive(format!("could not resolve {addr}")),
        };

        let mut stream = match TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2)) {
            Ok(stream) => stream,
            Err(err) => return ContentCheck::Inconclusive(format!("connect failed: {err}")),
        };
        if stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .is_err()
        {
            return ContentCheck::Inconclusive("could not set a read timeout".to_string());
        }

        let request = format!(
            "GET /hud-probe.html HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\
             User-Agent: keylink-studio-hud-focus-probe\r\n\r\n"
        );
        if let Err(err) = stream.write_all(request.as_bytes()) {
            return ContentCheck::Inconclusive(format!("write failed: {err}"));
        }

        let mut response = Vec::new();
        if let Err(err) = stream.read_to_end(&mut response) {
            // A read timeout (common with `Connection: close` once the
            // server has actually finished, since some servers don't
            // close promptly) still leaves whatever bytes already arrived
            // in `response` — good enough to check, so only bail if we
            // got nothing at all.
            if response.is_empty() {
                return ContentCheck::Inconclusive(format!("read failed: {err}"));
            }
        }

        let text = String::from_utf8_lossy(&response);
        let Some(status_line) = text.lines().next() else {
            return ContentCheck::Inconclusive("empty response".to_string());
        };
        if !status_line.contains("200") {
            return ContentCheck::Inconclusive(format!("unexpected status line: {status_line:?}"));
        }

        if text.contains("id=\"hud-probe-text\"") {
            ContentCheck::Confirmed
        } else {
            ContentCheck::MissingMarker
        }
    }

    /// Checks whether this build's configured dev server (if any) is
    /// actually reachable and actually serving the probe page, so the
    /// probe can VOID out up front instead of silently measuring a
    /// Chromium `ERR_CONNECTION_REFUSED` page.
    ///
    /// A debug build (`cargo run` without `--release`) loads the HUD from
    /// `tauri.conf.json`'s `build.devUrl`; if Vite's dev server isn't
    /// running, the HUD shows that error page instead of `hud-probe.html`,
    /// and every DOM-touching phase (P3's `set_hud_text` calls, P4/P0b's
    /// countdown text) silently measures nothing while still reporting a
    /// clean "0 events" — a real run hit exactly this and only the
    /// (separate) instrument-health VOID rule caught it, after the fact.
    ///
    /// Returns `Ok(None)` when everything checks out, `Ok(Some(warning))`
    /// when TCP reachability is confirmed but the (best-effort) content
    /// check could not run to completion — not fatal, see `ContentCheck::
    /// Inconclusive` — and `Err(reason)` when the run should VOID: either
    /// the dev server is unreachable at all, or it responded but the body
    /// didn't contain the probe page's marker element.
    ///
    /// Uses a raw TCP connect for the base reachability check (not an
    /// HTTP GET): `reqwest` is not a direct dependency of this crate, and
    /// a listening socket is a sufficient proxy for "Vite is up" without
    /// adding a dependency just for this. The content check
    /// (`fetch_hud_probe_html`) goes one step further with a minimal
    /// hand-rolled GET specifically because reachability alone was proven
    /// insufficient on real hardware: `WebviewWindow::url()` (see
    /// `build_log`'s "HUD webview URL" note) is not trustworthy evidence
    /// of what actually rendered, so this gives the probe an independent
    /// signal that isn't just "a TCP port answered."
    fn check_dev_server_reachable(app: &AppHandle) -> Result<Option<String>, String> {
        if !tauri::is_dev() {
            // `tauri::is_dev()` (`!cfg!(feature = "custom-protocol")`) is
            // the *exact* condition the webview itself uses to choose
            // between `devUrl` and `frontendDist` — that's why it's the
            // right gate here, not a proxy for it.
            //
            // Do NOT gate this on `app.config().build.dev_url.is_some()`
            // instead. `build.dev_url` is a static value straight out of
            // `tauri.conf.json`; it is `Some("http://localhost:5173/")`
            // regardless of build profile, including in a `--release`
            // build that will never touch a dev server. An earlier version
            // of this function checked `dev_url.is_some()` and, as a
            // result, VOIDed *every* release run before a single phase
            // ran — the probe looked like it silently did nothing, because
            // the only visible symptom (a VOID log line) was written to a
            // console that `--release` doesn't have. See the module doc's
            // "Run 3" note.
            return Ok(None);
        }

        let Some(dev_url) = app.config().build.dev_url.clone() else {
            // In dev mode but no devUrl configured; nothing to check.
            return Ok(None);
        };
        let Some(host) = dev_url.host_str() else {
            // Can't tell what to connect to; don't block the probe on a
            // check it cannot perform.
            return Ok(None);
        };
        let port = dev_url.port_or_known_default().unwrap_or(80);

        let addrs: Vec<_> = match (host, port).to_socket_addrs() {
            Ok(addrs) => addrs.collect(),
            Err(err) => {
                return Err(format!(
                    "the dev server address {host}:{port} (from tauri.conf.json's \
                     build.devUrl {dev_url}) could not be resolved: {err}, so the HUD would \
                     display a Chromium error page and P3 would measure nothing"
                ));
            }
        };

        let reachable = addrs
            .iter()
            .any(|addr| TcpStream::connect_timeout(addr, Duration::from_secs(2)).is_ok());

        if !reachable {
            return Err(format!(
                "the dev server at {dev_url} is not running, so the HUD would display a \
                 Chromium error page and P3 would measure nothing"
            ));
        }

        match fetch_hud_probe_html(host, port) {
            ContentCheck::Confirmed => Ok(None),
            ContentCheck::MissingMarker => Err(format!(
                "the dev server at {dev_url} responded to a GET for /hud-probe.html, but the \
                 response body did not contain `id=\"hud-probe-text\"` — the wrong page (very \
                 likely an error page) is being served, so the HUD would measure nothing"
            )),
            ContentCheck::Inconclusive(reason) => Ok(Some(format!(
                "could not verify /hud-probe.html's content via a raw HTTP GET ({reason}); TCP \
                 connectivity to the dev server at {dev_url} succeeded, so the probe is \
                 continuing — this is a limitation of the probe's minimal built-in HTTP client, \
                 not necessarily a real problem"
            ))),
        }
    }

    /// Shows a native `MessageBoxW` with the probe's headline result and
    /// the absolute path to its log file.
    ///
    /// A release build has no console (`main.rs`'s
    /// `windows_subsystem = "windows"`), so `println!` output is
    /// invisible — running `cargo run --release -p rawhid-host-tauri --
    /// --hud-focus-probe` would otherwise just return to the prompt with
    /// no visible sign anything happened, even though the probe ran to
    /// completion (or VOIDed) and wrote a log.
    ///
    /// Call this only after every phase (through P4b, the last one) has
    /// finished, or on an early-exit path that never runs any phase at
    /// all (e.g. the dev-server reachability check, or `HudWindow::create`
    /// failing) —
    /// never while phases are still executing. Showing a message box
    /// mid-run would itself steal foreground and contaminate the very
    /// measurement the probe exists to take.
    fn show_result_message_box(title: &str, text: &str) {
        let title_h = HSTRING::from(title);
        let text_h = HSTRING::from(text);
        unsafe {
            let _ = MessageBoxW(None, &text_h, &title_h, MB_OK | MB_ICONINFORMATION);
        }
    }

    /// Builds the absolute path this run's log will be written to.
    /// Computed once per exit path (early VOID, `HudWindow::create`
    /// failure, or normal completion) and threaded through to both the
    /// log body itself (so the path is visible in the file's own first
    /// lines) and the `println!`/`MessageBoxW` output, rather than letting
    /// `write_log` invent its own filename after the fact — that would let
    /// the timestamp embedded in the body drift from the actual filename.
    fn make_log_path() -> PathBuf {
        let filename = format!(
            "hud-focus-probe-{}.log",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        std::env::current_dir()
            .map(|dir| dir.join(&filename))
            .unwrap_or_else(|_| PathBuf::from(&filename))
    }

    /// Shared "which phase are we in" state, read by both instruments
    /// (`ForegroundProbe`'s hook callback and `ForegroundSampler`'s
    /// polling loop) so their events/transitions get tagged with the
    /// phase active at the moment they were observed. One `PhaseTracker`
    /// is created in `run()` and cloned into both instruments, so
    /// `run_phase` only has to call `set` once per phase to keep both in
    /// sync.
    #[derive(Clone)]
    struct PhaseTracker(Arc<Mutex<String>>);

    impl PhaseTracker {
        fn new(initial: &str) -> Self {
            Self(Arc::new(Mutex::new(initial.to_string())))
        }

        fn set(&self, phase: &str) {
            if let Ok(mut guard) = self.0.lock() {
                *guard = phase.to_string();
            }
        }

        fn get(&self) -> String {
            self.0
                .lock()
                .map(|guard| guard.clone())
                .unwrap_or_else(|_| "unknown".to_string())
        }
    }

    /// Entry point, run on a background thread by `run_hud_focus_probe()`
    /// so it never blocks the Tauri event loop (`setup()` must return
    /// promptly or the app never starts pumping messages).
    pub fn run(app: AppHandle) {
        let wall_clock_start = chrono::Local::now();
        let os_info = detect_os_version();

        // `Ok(None)`: nothing to report. `Ok(Some(warning))`: TCP
        // reachability was confirmed but the best-effort HTTP content
        // check (see `fetch_hud_probe_html`) could not complete — not
        // fatal, carried into the log as a note. `Err(reason)`: VOID
        // before any phase runs.
        let dev_server_warning = match check_dev_server_reachable(&app) {
            Ok(warning) => warning,
            Err(reason) => {
                let log_path = make_log_path();
                let overall_line = format!("Overall judgement: VOID ({reason})");

                let mut out = String::new();
                let _ = writeln!(out, "=== Keylink Studio HUD Focus Probe ===");
                let _ = writeln!(out, "Log file: {}", log_path.display());
                let _ = writeln!(out, "Started: {wall_clock_start}");
                let _ = writeln!(out, "OS: {os_info}");
                let _ = writeln!(out);
                let _ = writeln!(out, "{overall_line}");
                let _ = writeln!(out);
                // NOTE: do not suggest `cargo run --release` here. `is_dev()` is
                // `!cfg!(feature = "custom-protocol")` — a compile-time feature
                // of the `tauri` crate that only `cargo tauri build` turns on —
                // so `--release` is still a dev-mode binary and still needs the
                // dev server. A real run wasted a cycle on exactly that advice.
                let _ = writeln!(out, "Fix: start the Vite dev server first, then rerun.");
                let _ = writeln!(out);
                let _ = writeln!(out, "  Terminal 1:  cd ui && npm run dev");
                let _ = writeln!(
                    out,
                    "  Terminal 2:  cargo run -p rawhid-host-tauri -- --hud-focus-probe"
                );
                let _ = writeln!(out);
                let _ = writeln!(
                    out,
                    "  (`--release` does NOT help: Tauri's dev/production mode is decided by the \
                 `custom-protocol` cargo feature, not by the build profile, and this crate \
                 declares no passthrough for it.)"
                );
                write_log(&log_path, &out);
                // No phase has run yet at this point, so it is always safe —
                // never a mid-measurement foreground steal — to show this now.
                // This is also the exact path a real run hit with no visible
                // symptom at all in a release build; see the module doc.
                show_result_message_box(
                    "Keylink Studio HUD Focus Probe",
                    &format!("{overall_line}\n\nLog: {}", log_path.display()),
                );
                // Leave no lingering invisible process — see `app.exit(0)`
                // at the end of the normal path for why.
                app.exit(0);
                return;
            }
        };

        let start = Instant::now();
        let phase_tracker = PhaseTracker::new("P0-startup");
        let hook = ForegroundProbe::start(start, phase_tracker.clone());
        let sampler = ForegroundSampler::start(start, phase_tracker.clone());

        let hud =
            match HudWindow::create(&app, "hud-probe", WebviewUrl::App("hud-probe.html".into())) {
                Ok(hud) => hud,
                Err(err) => {
                    let log_path = make_log_path();
                    let log = build_fatal_log(&log_path, &wall_clock_start, &os_info, &err);
                    write_log(&log_path, &log);
                    // No phase has run yet here either — safe to show
                    // immediately, same reasoning as the dev-server VOID
                    // path above.
                    show_result_message_box(
                        "Keylink Studio HUD Focus Probe",
                        &format!(
                            "Overall judgement: VOID (probe could not run)\n\nLog: {}",
                            log_path.display()
                        ),
                    );
                    // Leave no lingering invisible process — see
                    // `app.exit(0)` at the end of the normal path for why.
                    app.exit(0);
                    return;
                }
            };

        // WebView2 initialization settle time before taking a baseline
        // reading, per the probe design: the first paint / first navigation
        // of a freshly created WebView2 control is exactly the kind of
        // one-off event that could plausibly (and wrongly) grab focus, so
        // give it room to finish before P0b/P1 start measuring against it.
        thread::sleep(Duration::from_secs(2));
        let p0_snapshot = focus_snapshot();
        let p0_hook_events = hook.events_since(Duration::ZERO);
        let p0_sampler_events = sampler.events_since(Duration::ZERO);
        // Recorded so a mismatch between "what we asked for" and "what the
        // webview actually loaded" (e.g. an error page) is visible in the
        // log rather than only inferred from downstream symptoms.
        let hud_url = match hud.window().url() {
            Ok(url) => url.to_string(),
            Err(err) => format!("<error reading webview URL: {err}>"),
        };

        let (hud_x, hud_y) = bottom_right_position(&hud, HUD_WIDTH, HUD_HEIGHT);

        let mut phases: Vec<PhaseRecord> = Vec::new();

        // --- P0b: instrument precheck ---------------------------------
        // Run *before* the measured phases (P1-P3) so both ends of the
        // window we actually need to trust are bracketed by a confirmed-
        // alive instrument reading — see the module doc's "Runs 4 and 5"
        // note for why P4's click can no longer be allowed to sit between
        // the two checks the way the old P5 postcheck did.
        let pre_precheck_hwnd = p0_snapshot.foreground_hwnd;
        let mut p0b_check: Option<InstrumentCheckResult> = None;
        let mut p0b_quiescence: Option<QuiescenceOutcome> = None;
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P0b-instrument-precheck",
            "P0b 計測器事前確認",
            || {
                p0b_check = Some(run_instrument_check_with_retries(
                    &hud,
                    start,
                    &hook,
                    &sampler,
                    "P0b-instrument-precheck",
                    pre_precheck_hwnd,
                ));
                wait_for_foreground_return(
                    &hud,
                    hud_x,
                    hud_y,
                    HUD_WIDTH,
                    HUD_HEIGHT,
                    pre_precheck_hwnd,
                    Duration::from_secs(5),
                );
                // Absorb the "back to editor" cleanup transition into this
                // phase's own window instead of letting it leak into P1 —
                // see `wait_for_quiescence`'s doc comment for the exact
                // real-hardware numbers that made this necessary.
                p0b_quiescence = Some(wait_for_quiescence(
                    &hook,
                    &sampler,
                    start,
                    "P0b-instrument-precheck",
                    pre_precheck_hwnd,
                    QUIESCENCE_MIN_STABLE,
                    QUIESCENCE_TIMEOUT,
                ));
                TypistSummary::none()
            },
        ));
        if let Some(phase) = phases.last_mut() {
            phase.instrument_check = p0b_check;
            phase.quiescence = p0b_quiescence;
        }

        // --- P1: first show ---------------------------------------------
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P1-first-show",
            "P1 初回表示",
            || {
                hud.show_at(hud_x, hud_y, HUD_WIDTH, HUD_HEIGHT);
                thread::sleep(Duration::from_millis(500));
                hud.hide();
                thread::sleep(Duration::from_millis(500));
                TypistSummary::none()
            },
        ));

        // --- P2: repeated show/hide while typing -------------------------
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P2-repeat",
            "P2 反復",
            || {
                let typist = Typist::start();
                for _ in 0..50 {
                    hud.show_at(hud_x, hud_y, HUD_WIDTH, HUD_HEIGHT);
                    thread::sleep(Duration::from_millis(200));
                    hud.hide();
                    thread::sleep(Duration::from_millis(200));
                }
                let sent = typist.stop();
                TypistSummary {
                    sent,
                    phase_label: "P2-repeat".to_string(),
                }
            },
        ));

        // --- P3: content updates while shown ------------------------------
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P3-content-update",
            "P3 内容更新",
            || {
                hud.show_at(hud_x, hud_y, HUD_WIDTH, HUD_HEIGHT);
                for i in 0..20u32 {
                    let text = format!(
                        "P3 update #{} / 20 @ {}ms",
                        i + 1,
                        start.elapsed().as_millis()
                    );
                    set_hud_text(&hud, &text);
                    thread::sleep(Duration::from_millis(200));
                }
                hud.hide();
                TypistSummary::none()
            },
        ));

        // --- P3b: instrument postcheck (moved ahead of P4) -------------
        // Deliberately activates the HUD window (see the doc comment on
        // `show_activating_for_instrument_check`), retried up to 3 times
        // via `run_instrument_check_with_retries`, so both instruments have
        // a confirmed positive case bracketing P1-P3 from the other side
        // too (P0b covers the start). This MUST run before P4: P4's click
        // is known (two real runs, see the module doc) to leave the hook
        // silent afterward, so putting the postcheck after P4 like the old
        // P5 did was structurally guaranteed to VOID every run. If either
        // instrument never fires across 3 attempts at either P0b or P3b —
        // `instrument_health_problems` VOIDs the whole run: a P1-P3 "0
        // events" reading from an instrument that cannot be shown alive is
        // meaningless.
        let pre_p3b_hwnd = focus_snapshot().foreground_hwnd;
        let mut p3b_check: Option<InstrumentCheckResult> = None;
        let mut p3b_quiescence: Option<QuiescenceOutcome> = None;
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P3b-instrument-postcheck",
            "P3b 計測器事後確認",
            || {
                p3b_check = Some(run_instrument_check_with_retries(
                    &hud,
                    start,
                    &hook,
                    &sampler,
                    "P3b-instrument-postcheck",
                    pre_p3b_hwnd,
                ));
                // Same human-visible last resort as P0b: P4 needs a
                // non-HUD (ideally the editor) foreground to start from,
                // and the retry function's own restore attempt can be
                // denied by Windows' foreground-lock rules.
                wait_for_foreground_return(
                    &hud,
                    hud_x,
                    hud_y,
                    HUD_WIDTH,
                    HUD_HEIGHT,
                    pre_p3b_hwnd,
                    Duration::from_secs(5),
                );
                // Same leak this closes for P0b -> P1: without this, the
                // "back to editor" transition lands in P4 instead of here
                // — see `wait_for_quiescence`'s doc comment.
                p3b_quiescence = Some(wait_for_quiescence(
                    &hook,
                    &sampler,
                    start,
                    "P3b-instrument-postcheck",
                    pre_p3b_hwnd,
                    QUIESCENCE_MIN_STABLE,
                    QUIESCENCE_TIMEOUT,
                ));
                TypistSummary::none()
            },
        ));
        if let Some(phase) = phases.last_mut() {
            phase.instrument_check = p3b_check;
            phase.quiescence = p3b_quiescence;
        }

        // --- P4: click sampling (now the last scored phase) ----------------
        // Run last on purpose: two real runs showed the hook's last
        // firing of the *entire run* was always P4's click, silent ever
        // after — see the module doc. P4 itself doesn't need an
        // instrument-health proof the way P0b/P3b do, because a positive
        // hook detection here (see C7's PASS/FAIL rule below) is itself
        // direct evidence the hook was working *at the moment of the
        // click* — a positive reading proves the instrument was alive,
        // the same way P5 used to prove it deliberately. Only a negative
        // (zero) reading would need independent confirmation the
        // instrument wasn't just dead, and P0b/P3b already provide that
        // for the whole P1-P3 window this negative would otherwise cast
        // doubt on.
        //
        // `p4_click_detected` is written inside the closure (by mutable
        // reference) and read back once `run_phase` returns; see
        // `wait_for_hud_click` for how the click itself is confirmed.
        let mut p4_click_detected: Option<bool> = None;
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P4-click",
            "P4 クリック",
            || {
                hud.show_at(hud_x, hud_y, HUD_WIDTH, HUD_HEIGHT);
                let detected = wait_for_hud_click(
                    &hud,
                    hud_x,
                    hud_y,
                    HUD_WIDTH,
                    HUD_HEIGHT,
                    Duration::from_secs(20),
                );
                p4_click_detected = Some(detected);
                hud.hide();
                TypistSummary::none()
            },
        ));
        if let Some(phase) = phases.last_mut() {
            phase.click_detected = p4_click_detected;
        }

        // --- P4b: post-click instrument spot-check (informational only) ----
        // A single activation, not retried, and explicitly excluded from
        // the overall judgement (see the `is_p4b` guard in `build_log`'s
        // judgement loop) — its only purpose is to record, for whoever
        // reads the log, whether the click's known hook-silencing effect
        // (module doc) shows up on this particular run, so the C7 line can
        // carry a "(hook may be degraded after the click — see P4b)" note
        // when it does.
        phases.push(run_phase(
            start,
            &phase_tracker,
            &hook,
            &sampler,
            "P4b-instrument-final",
            "P4b 計測器最終確認（参考）",
            || {
                hud.show_activating_for_instrument_check();
                thread::sleep(Duration::from_secs(1));
                hud.hide();
                TypistSummary::none()
            },
        ));

        // Best-effort cleanup; the probe process exits shortly after logging.
        hud.hide();

        let log_path = make_log_path();
        let log = build_log(
            &log_path,
            &wall_clock_start,
            &os_info,
            &hud_url,
            dev_server_warning.as_deref(),
            &p0_snapshot,
            &p0_hook_events,
            &p0_sampler_events,
            &phases,
        );
        write_log(&log_path, &log);

        // Every phase through P4b has finished by this point, so showing
        // the dialog now cannot contaminate any measurement — see
        // `show_result_message_box`'s doc comment.
        let overall_line = log
            .lines()
            .find(|line| line.starts_with("Overall judgement: "))
            .unwrap_or("Overall judgement: UNKNOWN (could not locate judgement line in log)");
        show_result_message_box(
            "Keylink Studio HUD Focus Probe",
            &format!("{overall_line}\n\nLog: {}", log_path.display()),
        );

        // The probe is a one-shot measurement tool, not an app: once the
        // result is on screen there is nothing left for the event loop to
        // do. Without this the process lingers with two hidden windows and
        // no taskbar presence (`skip_taskbar`), so it is invisible but
        // still holding a lock on its own .exe — which broke three
        // subsequent rebuilds and contributed to a run looking like it
        // "did nothing". Safe from this worker thread: `AppHandle::exit`
        // is explicitly cross-thread.
        app.exit(0);
    }

    /// Runs one phase's body between a before/after `focus_snapshot()` and
    /// wall-clock timestamp, with the shared phase name set for the
    /// duration so both instruments' callbacks tag events correctly.
    #[allow(clippy::too_many_arguments)]
    fn run_phase(
        start: Instant,
        phase_tracker: &PhaseTracker,
        hook: &ForegroundProbe,
        sampler: &ForegroundSampler,
        phase_id: &str,
        phase_label: &str,
        body: impl FnOnce() -> TypistSummary,
    ) -> PhaseRecord {
        phase_tracker.set(phase_id);
        let start_ts = chrono::Local::now();
        let before = focus_snapshot();
        let window_start = start.elapsed();

        let typist = body();

        let window_end = start.elapsed();
        let after = focus_snapshot();
        let end_ts = chrono::Local::now();

        let hook_events = hook.events_between(window_start, window_end, phase_id);
        let sampler_events = sampler.events_between(window_start, window_end, phase_id);

        PhaseRecord {
            id: phase_id.to_string(),
            label: phase_label.to_string(),
            start_ts,
            end_ts,
            before,
            after,
            hook_events,
            sampler_events,
            typist,
            click_detected: None,
            instrument_check: None,
            quiescence: None,
        }
    }

    // ---------------------------------------------------------------------
    // Shared event storage helpers, used by both instruments below.
    // ---------------------------------------------------------------------

    #[derive(Debug, Clone)]
    struct ForegroundEvent {
        elapsed_ms: u128,
        hwnd: isize,
        pid: u32,
        title: String,
        phase: String,
    }

    fn events_since_impl(
        events: &Mutex<Vec<ForegroundEvent>>,
        since: Duration,
    ) -> Vec<ForegroundEvent> {
        let since_ms = since.as_millis();
        events
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| event.elapsed_ms >= since_ms)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Events recorded in `[since, until]` and tagged with `phase_id`. The
    /// phase-name filter is the primary signal (it is set on this exact
    /// window); the time bound is a belt-and-suspenders guard in case a
    /// callback is still in flight right at a phase boundary.
    fn events_between_impl(
        events: &Mutex<Vec<ForegroundEvent>>,
        since: Duration,
        until: Duration,
        phase_id: &str,
    ) -> Vec<ForegroundEvent> {
        let since_ms = since.as_millis();
        let until_ms = until.as_millis();
        events
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| {
                        event.phase == phase_id
                            && event.elapsed_ms >= since_ms
                            && event.elapsed_ms <= until_ms
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    fn window_title(hwnd: HWND) -> String {
        let len = unsafe { GetWindowTextLengthW(hwnd) };
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let copied = unsafe { GetWindowTextW(hwnd, &mut buf) };
        if copied <= 0 {
            return String::new();
        }
        buf.truncate(copied as usize);
        String::from_utf16_lossy(&buf)
    }

    // ---------------------------------------------------------------------
    // ForegroundProbe: event-driven measurement instrument (the hook).
    // ---------------------------------------------------------------------

    struct HookState {
        start: Instant,
        events: Arc<Mutex<Vec<ForegroundEvent>>>,
        phase: PhaseTracker,
    }

    thread_local! {
        static PROBE_STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
    }

    struct ForegroundProbe {
        thread_id: u32,
        join: Option<JoinHandle<()>>,
        events: Arc<Mutex<Vec<ForegroundEvent>>>,
    }

    impl ForegroundProbe {
        fn start(start: Instant, phase: PhaseTracker) -> Self {
            let events = Arc::new(Mutex::new(Vec::new()));
            let (id_tx, id_rx) = mpsc::channel::<u32>();

            let state = HookState {
                start,
                events: events.clone(),
                phase,
            };

            let join = thread::spawn(move || {
                PROBE_STATE.with(|cell| *cell.borrow_mut() = Some(state));

                // Deliberately WITHOUT `WINEVENT_SKIPOWNPROCESS`.
                // `foreground.rs` (the production layer-switch watcher) sets
                // that flag because it only cares about *other*
                // applications taking focus. Here the failure mode under
                // test is our own HUD window (this process) stealing
                // focus, so excluding our own process from the hook would
                // silently discard exactly the event this probe exists to
                // catch, producing a guaranteed false PASS.
                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT,
                    )
                };

                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                let _ = id_tx.send(thread_id);

                if hook.is_invalid() {
                    PROBE_STATE.with(|cell| *cell.borrow_mut() = None);
                    return;
                }

                let mut msg = MSG::default();
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWinEvent(hook);
                }
                PROBE_STATE.with(|cell| *cell.borrow_mut() = None);
            });

            let thread_id = id_rx.recv().unwrap_or(0);
            Self {
                thread_id,
                join: Some(join),
                events,
            }
        }

        fn events_since(&self, since: Duration) -> Vec<ForegroundEvent> {
            events_since_impl(&self.events, since)
        }

        fn events_between(
            &self,
            since: Duration,
            until: Duration,
            phase_id: &str,
        ) -> Vec<ForegroundEvent> {
            events_between_impl(&self.events, since, until, phase_id)
        }
    }

    impl Drop for ForegroundProbe {
        fn drop(&mut self) {
            if self.thread_id != 0 {
                unsafe {
                    let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
                }
            }
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    unsafe extern "system" fn win_event_proc(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        _id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        PROBE_STATE.with(|cell| {
            if let Some(state) = cell.borrow().as_ref() {
                let elapsed_ms = state.start.elapsed().as_millis();
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                let title = window_title(hwnd);
                let phase = state.phase.get();
                if let Ok(mut events) = state.events.lock() {
                    events.push(ForegroundEvent {
                        elapsed_ms,
                        hwnd: hwnd.0 as isize,
                        pid,
                        title,
                        phase,
                    });
                }
            }
        });
    }

    // ---------------------------------------------------------------------
    // ForegroundSampler: polling measurement instrument (the cross-check).
    // ---------------------------------------------------------------------

    /// Independent second measurement instrument: polls
    /// `GetForegroundWindow()` every 20ms on its own thread and records a
    /// transition whenever the HWND differs from the previous sample.
    ///
    /// This exists because `ForegroundProbe`'s
    /// `SetWinEventHook(..., WINEVENT_OUTOFCONTEXT)` hook is a known-lossy
    /// delivery mechanism: on a second real-hardware run, the hook
    /// recorded the P4 HUD activation but missed BOTH the
    /// return-to-editor transition that must have happened between P4 and
    /// P5 (P4's "after" and P5's "before" snapshot were both the editor)
    /// AND the P5 `SetForegroundWindow`-driven transition to the HUD
    /// itself (confirmed by P5's "after" snapshot) — two guaranteed real
    /// transitions, zero hook events for either. `WINEVENT_OUTOFCONTEXT`
    /// is documented as best-effort delivery: if the hook thread's message
    /// loop can't keep up, Windows silently drops the notification rather
    /// than queuing it. A 20ms poll cannot silently drop anything the same
    /// way — the only way for it to miss a transition is for that
    /// transition to both start and revert within one 20ms window — which
    /// is why the judgement logic in `build_log` treats the sampler as
    /// authoritative for pass/fail and the hook as a secondary
    /// cross-check only (see the per-phase "hook/sampler disagreement"
    /// note).
    struct ForegroundSampler {
        stop: Arc<AtomicBool>,
        join: Option<JoinHandle<()>>,
        events: Arc<Mutex<Vec<ForegroundEvent>>>,
    }

    impl ForegroundSampler {
        fn start(start: Instant, phase: PhaseTracker) -> Self {
            let events = Arc::new(Mutex::new(Vec::new()));
            let stop = Arc::new(AtomicBool::new(false));
            let events_bg = events.clone();
            let stop_bg = stop.clone();

            let join = thread::spawn(move || {
                let mut last_hwnd: Option<isize> = None;
                while !stop_bg.load(Ordering::SeqCst) {
                    let hwnd = unsafe { GetForegroundWindow() };
                    let hwnd_raw = hwnd.0 as isize;
                    if last_hwnd != Some(hwnd_raw) {
                        // Only record actual *transitions*; the very first
                        // sample just establishes the baseline and is not
                        // itself a change.
                        if last_hwnd.is_some() {
                            let elapsed_ms = start.elapsed().as_millis();
                            let mut pid = 0u32;
                            unsafe {
                                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                            }
                            let title = window_title(hwnd);
                            let phase_name = phase.get();
                            if let Ok(mut events) = events_bg.lock() {
                                events.push(ForegroundEvent {
                                    elapsed_ms,
                                    hwnd: hwnd_raw,
                                    pid,
                                    title,
                                    phase: phase_name,
                                });
                            }
                        }
                        last_hwnd = Some(hwnd_raw);
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            });

            Self {
                stop,
                join: Some(join),
                events,
            }
        }

        fn events_since(&self, since: Duration) -> Vec<ForegroundEvent> {
            events_since_impl(&self.events, since)
        }

        fn events_between(
            &self,
            since: Duration,
            until: Duration,
            phase_id: &str,
        ) -> Vec<ForegroundEvent> {
            events_between_impl(&self.events, since, until, phase_id)
        }
    }

    impl Drop for ForegroundSampler {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }

    // ---------------------------------------------------------------------
    // focus_snapshot(): point-in-time foreground/focus/caret state.
    // ---------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq)]
    struct FocusSnapshot {
        foreground_hwnd: isize,
        foreground_title: String,
        foreground_pid: u32,
        gui_active_hwnd: isize,
        gui_focus_hwnd: isize,
        gui_caret_hwnd: isize,
        gui_caret_rect: (i32, i32, i32, i32),
    }

    impl FocusSnapshot {
        /// Whether *activation* state is unchanged between `self` and
        /// `other` — deliberately excludes `gui_caret_rect`.
        ///
        /// Do not add `gui_caret_rect` back into this comparison. It was
        /// tried and it produced a false FAIL: in the P2 measurement run
        /// that motivated this method, the caret's *x* coordinate moved
        /// ~1664px over 201 typed characters (~8.3px/char) while every
        /// other field — including the caret's *owning* HWND
        /// (`gui_caret_hwnd`) and its y position — stayed identical. That
        /// is exactly what a caret is supposed to do while the probe's own
        /// `Typist` types into the still-focused foreground window: P2 is
        /// the one phase that *deliberately* generates keystrokes, so
        /// `rcCaret` moving is expected, correct behavior, not evidence of
        /// a focus steal. Comparing the full snapshot (including
        /// `rcCaret`) conflates "the caret advanced because we typed" with
        /// "something stole focus", which is backwards — the former is
        /// evidence *for* a pass. See `caret_evidence` for how `rcCaret`
        /// is still surfaced, as an informational line rather than part of
        /// this verdict.
        fn activation_unchanged(&self, other: &Self) -> bool {
            self.foreground_hwnd == other.foreground_hwnd
                && self.foreground_pid == other.foreground_pid
                && self.foreground_title == other.foreground_title
                && self.gui_active_hwnd == other.gui_active_hwnd
                && self.gui_focus_hwnd == other.gui_focus_hwnd
                && self.gui_caret_hwnd == other.gui_caret_hwnd
        }
    }

    /// Informational (non-verdict) evidence line describing how far the
    /// caret moved during a phase that ran the typist, e.g.:
    /// "x moved 1664px over 201 typed characters (8.28px/char), y
    /// unchanged: true". This exists to *support* a pass — it shows
    /// keystrokes landed in the expected foreground window and advanced
    /// its caret by a plausible per-character amount — not to gate one.
    /// Returns `None` when no characters were typed during the phase (the
    /// common case for every phase except P2).
    fn caret_evidence(
        before: &FocusSnapshot,
        after: &FocusSnapshot,
        typed: usize,
    ) -> Option<String> {
        if typed == 0 {
            return None;
        }
        let (before_left, before_top, _, before_bottom) = before.gui_caret_rect;
        let (after_left, after_top, _, after_bottom) = after.gui_caret_rect;
        let dx = after_left - before_left;
        let y_unchanged = before_top == after_top && before_bottom == after_bottom;
        let px_per_char = dx as f64 / typed as f64;
        Some(format!(
            "Caret evidence (informational, not a verdict): x moved {dx}px over {typed} typed \
             characters ({px_per_char:.2}px/char), y unchanged: {y_unchanged} \
             (before rcCaret={:?}, after rcCaret={:?}) — supports \"keystrokes reached the \
             expected window\", it does not gate PASS/FAIL",
            before.gui_caret_rect, after.gui_caret_rect
        ))
    }

    fn focus_snapshot() -> FocusSnapshot {
        let fg = unsafe { GetForegroundWindow() };
        let foreground_title = window_title(fg);
        let mut foreground_pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(fg, Some(&mut foreground_pid));
        }

        // `GUITHREADINFO` requires `cbSize` to be set before the call; the
        // rest of the struct is populated by `GetGUIThreadInfo`.
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        // Thread id 0 queries the foreground thread's GUI state.
        let ok = unsafe { GetGUIThreadInfo(0, &mut info) };

        let (gui_active_hwnd, gui_focus_hwnd, gui_caret_hwnd, gui_caret_rect) = if ok.is_ok() {
            (
                info.hwndActive.0 as isize,
                info.hwndFocus.0 as isize,
                info.hwndCaret.0 as isize,
                rect_tuple(info.rcCaret),
            )
        } else {
            (0, 0, 0, (0, 0, 0, 0))
        };

        FocusSnapshot {
            foreground_hwnd: fg.0 as isize,
            foreground_title,
            foreground_pid,
            gui_active_hwnd,
            gui_focus_hwnd,
            gui_caret_hwnd,
            gui_caret_rect,
        }
    }

    fn rect_tuple(rect: RECT) -> (i32, i32, i32, i32) {
        (rect.left, rect.top, rect.right, rect.bottom)
    }

    // ---------------------------------------------------------------------
    // Typist: background SendInput driver.
    // ---------------------------------------------------------------------

    const TYPIST_CHARS: &[u8] = b"abcdefghij";

    struct Typist {
        stop: Arc<AtomicBool>,
        count: Arc<AtomicUsize>,
        join: Option<JoinHandle<()>>,
    }

    impl Typist {
        fn start() -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let count = Arc::new(AtomicUsize::new(0));
            let stop_bg = stop.clone();
            let count_bg = count.clone();

            let join = thread::spawn(move || {
                let mut i = 0usize;
                while !stop_bg.load(Ordering::SeqCst) {
                    let ch = TYPIST_CHARS[i % TYPIST_CHARS.len()] as char;
                    send_char(ch);
                    count_bg.fetch_add(1, Ordering::SeqCst);
                    i += 1;
                    thread::sleep(Duration::from_millis(100));
                }
            });

            Self {
                stop,
                count,
                join: Some(join),
            }
        }

        /// Signals the background sender to stop, joins it, and returns the
        /// total number of characters sent.
        fn stop(mut self) -> usize {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
            self.count.load(Ordering::SeqCst)
        }
    }

    /// Sends one keypress (down + up) for `ch` via `SendInput`, using
    /// `wScan` + `KEYEVENTF_UNICODE` (not `wVk`) so the character is
    /// delivered independent of the active keyboard layout.
    fn send_char(ch: char) {
        let code = ch as u16;
        let ki_down = KEYBDINPUT {
            wVk: VIRTUAL_KEY(0),
            wScan: code,
            dwFlags: KEYEVENTF_UNICODE,
            time: 0,
            dwExtraInfo: 0,
        };
        let mut ki_up = ki_down;
        ki_up.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;

        let down = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: ki_down },
        };
        let up = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: ki_up },
        };

        let inputs = [down, up];
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }

    struct TypistSummary {
        sent: usize,
        phase_label: String,
    }

    impl TypistSummary {
        fn none() -> Self {
            Self {
                sent: 0,
                phase_label: String::new(),
            }
        }
    }

    // ---------------------------------------------------------------------
    // Logging / judgement.
    // ---------------------------------------------------------------------

    struct PhaseRecord {
        id: String,
        label: String,
        start_ts: chrono::DateTime<chrono::Local>,
        end_ts: chrono::DateTime<chrono::Local>,
        before: FocusSnapshot,
        after: FocusSnapshot,
        hook_events: Vec<ForegroundEvent>,
        sampler_events: Vec<ForegroundEvent>,
        typist: TypistSummary,
        /// Only meaningful for the P4 phase: `Some(true)` when
        /// `wait_for_hud_click` confirmed a click landed on the HUD,
        /// `Some(false)` when it timed out without one, `None` for every
        /// other phase (which don't attempt click detection at all).
        click_detected: Option<bool>,
        /// Only meaningful for P0b/P3b: the retry attempts and per-instrument
        /// outcome from `run_instrument_check_with_retries`. `None` for
        /// every other phase (which don't retry an activation at all).
        instrument_check: Option<InstrumentCheckResult>,
        /// Only meaningful for P0b/P3b: the outcome of the trailing
        /// `wait_for_quiescence` call. `None` for every other phase (which
        /// don't do a quiescence wait at all).
        quiescence: Option<QuiescenceOutcome>,
    }

    fn bottom_right_position(hud: &HudWindow, w: i32, h: i32) -> (i32, i32) {
        match hud.window().primary_monitor() {
            Ok(Some(monitor)) => {
                let size = monitor.size();
                let pos = monitor.position();
                let x = pos.x + size.width as i32 - w - MONITOR_MARGIN;
                let y = pos.y + size.height as i32 - h - MONITOR_MARGIN;
                (x.max(pos.x), y.max(pos.y))
            }
            _ => (100, 100),
        }
    }

    fn detect_os_version() -> String {
        // No `Win32_System_SystemInformation` feature is enabled for this
        // crate, so version info is obtained the portable way: shelling out
        // to `cmd /c ver` rather than adding a new dependency/feature.
        match std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output()
        {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
            Ok(output) => format!("unknown (ver exited with {:?})", output.status.code()),
            Err(err) => format!("unknown (failed to run `cmd /c ver`: {err})"),
        }
    }

    fn write_log(path: &Path, content: &str) {
        match fs::write(path, content) {
            Ok(()) => {
                #[cfg(debug_assertions)]
                println!("[hud-focus-probe] wrote {}", path.display());
            }
            Err(err) => {
                // In a release build (no `debug_assertions`, hence no
                // console per `main.rs`'s `windows_subsystem`) `err` is
                // otherwise unused; `show_result_message_box`'s caller
                // still surfaces the overall judgement via `MessageBoxW`
                // regardless of whether the file write itself succeeded.
                let _ = &err;
                #[cfg(debug_assertions)]
                println!(
                    "[hud-focus-probe] FAILED to write {}: {err}",
                    path.display()
                );
            }
        }

        #[cfg(debug_assertions)]
        println!("{content}");
    }

    fn build_fatal_log(
        log_path: &Path,
        wall_clock_start: &chrono::DateTime<chrono::Local>,
        os_info: &str,
        err: &str,
    ) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "=== Keylink Studio HUD Focus Probe ===");
        let _ = writeln!(out, "Log file: {}", log_path.display());
        let _ = writeln!(out, "Started: {wall_clock_start}");
        let _ = writeln!(out, "OS: {os_info}");
        let _ = writeln!(out);
        let _ = writeln!(out, "FATAL: could not create HUD window: {err}");
        let _ = writeln!(out);
        let _ = writeln!(out, "Overall judgement: VOID (probe could not run)");
        out
    }

    /// Builds the list of problems (if any) with the pair of
    /// instrument-health checks — P0b before the measured phases, P3b
    /// after (moved ahead of P4 — see the module doc's "Runs 4 and 5"
    /// note) — now that each check itself retries up to 3 times (see
    /// `run_instrument_check_with_retries`). Each instrument must have
    /// fired at least once across *its own* retries at *both* P0b and P3b
    /// for a run's P1-P3 results to be trustworthy: see the VOID branch of
    /// `build_log`'s overall judgement, which treats a non-empty result
    /// here as unconditionally VOID regardless of how clean P1-P3 looked.
    fn instrument_health_problems(
        p0b: Option<&InstrumentCheckResult>,
        p3b: Option<&InstrumentCheckResult>,
    ) -> Vec<String> {
        let mut problems = Vec::new();

        match p0b {
            Some(check) => {
                if !check.hook_alive {
                    problems.push(format!(
                        "hook did not fire in {} P0b (precheck) attempts",
                        check.attempts.len()
                    ));
                }
                if !check.sampler_alive {
                    problems.push(format!(
                        "sampler did not fire in {} P0b (precheck) attempts",
                        check.attempts.len()
                    ));
                }
            }
            None => problems.push("P0b instrument precheck did not run".to_string()),
        }

        match p3b {
            Some(check) => {
                if !check.hook_alive {
                    problems.push(format!(
                        "hook did not fire in {} P3b (postcheck) attempts",
                        check.attempts.len()
                    ));
                }
                if !check.sampler_alive {
                    problems.push(format!(
                        "sampler did not fire in {} P3b (postcheck) attempts",
                        check.attempts.len()
                    ));
                }
            }
            None => problems.push("P3b instrument postcheck did not run".to_string()),
        }

        problems
    }

    /// Formats the "Instrument health" summary line: `VOID (...)` when
    /// `problems` is non-empty, otherwise `OK` with the attempt number
    /// each instrument first fired on at each check.
    fn instrument_health_line(
        p0b: Option<&InstrumentCheckResult>,
        p3b: Option<&InstrumentCheckResult>,
        problems: &[String],
    ) -> String {
        if !problems.is_empty() {
            return format!("VOID ({})", problems.join("; "));
        }

        fn attempt_str(
            check: Option<&InstrumentCheckResult>,
            first: impl Fn(&InstrumentCheckResult) -> Option<u32>,
        ) -> String {
            check
                .and_then(first)
                .map(|n| n.to_string())
                .unwrap_or_else(|| "?".to_string())
        }

        format!(
            "OK (hook: fired at P0b attempt {} and P3b attempt {}; sampler: fired at P0b \
             attempt {} and P3b attempt {})",
            attempt_str(p0b, |c| c.hook_first_attempt),
            attempt_str(p3b, |c| c.hook_first_attempt),
            attempt_str(p0b, |c| c.sampler_first_attempt),
            attempt_str(p3b, |c| c.sampler_first_attempt),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_log(
        log_path: &Path,
        wall_clock_start: &chrono::DateTime<chrono::Local>,
        os_info: &str,
        hud_url: &str,
        dev_server_warning: Option<&str>,
        p0_snapshot: &FocusSnapshot,
        p0_hook_events: &[ForegroundEvent],
        p0_sampler_events: &[ForegroundEvent],
        phases: &[PhaseRecord],
    ) -> String {
        let mut out = String::new();

        let _ = writeln!(out, "=== Keylink Studio HUD Focus Probe ===");
        let _ = writeln!(out, "Log file: {}", log_path.display());
        let _ = writeln!(out, "Started: {wall_clock_start}");
        let _ = writeln!(out, "OS: {os_info}");
        if let Some(warning) = dev_server_warning {
            let _ = writeln!(out, "Dev-server content check warning: {warning}");
        }
        let _ = writeln!(out);

        let _ = writeln!(out, "--- Phase P0: startup (informational, not scored) ---");
        let _ = writeln!(
            out,
            "Window created hidden, waited 2s for WebView2 init, then took baseline snapshot."
        );
        let _ = writeln!(out, "HUD webview URL: {hud_url}");
        let _ = writeln!(
            out,
            "Note: WebviewWindow::url() is not reliable evidence of what actually loaded in \
             Tauri v2 — a real run saw this return \"about:blank\" while hud-probe.html was \
             demonstrably rendering correctly (P4's Japanese countdown text was visible and \
             updating). Treat this URL as informational only, not as proof of a blank/failed \
             page load."
        );
        let _ = writeln!(out, "Baseline snapshot: {p0_snapshot:?}");
        let _ = writeln!(
            out,
            "Foreground events during startup+wait (hook)    : {}",
            p0_hook_events.len()
        );
        for event in p0_hook_events {
            let _ = writeln!(out, "  {}", format_event(event));
        }
        let _ = writeln!(
            out,
            "Foreground events during startup+wait (sampler) : {}",
            p0_sampler_events.len()
        );
        for event in p0_sampler_events {
            let _ = writeln!(out, "  {}", format_event(event));
        }
        let _ = writeln!(out);

        for phase in phases {
            let _ = writeln!(out, "--- Phase {}: {} ---", phase.id, phase.label);
            let _ = writeln!(out, "Start: {}", phase.start_ts);
            let _ = writeln!(out, "End:   {}", phase.end_ts);
            let _ = writeln!(out, "Before: {:?}", phase.before);
            let _ = writeln!(out, "After:  {:?}", phase.after);
            let _ = writeln!(
                out,
                "Activation unchanged (excludes gui_caret_rect — see FocusSnapshot::activation_unchanged): {}",
                phase.before.activation_unchanged(&phase.after)
            );
            let _ = writeln!(
                out,
                "Foreground events (hook)    : {}",
                phase.hook_events.len()
            );
            for event in &phase.hook_events {
                let _ = writeln!(out, "  {}", format_event(event));
            }
            let _ = writeln!(
                out,
                "Foreground events (sampler) : {}",
                phase.sampler_events.len()
            );
            for event in &phase.sampler_events {
                let _ = writeln!(out, "  {}", format_event(event));
            }
            if phase.hook_events.len() != phase.sampler_events.len() {
                let _ = writeln!(
                    out,
                    "hook/sampler disagreement (hook is known to drop WINEVENT_OUTOFCONTEXT \
                     deliveries; sampler is authoritative)"
                );
            }
            if phase.typist.sent > 0 {
                let _ = writeln!(
                    out,
                    "Typist: sent {} characters during phase {} (pattern \"abcdefghij\" repeated, 100ms interval)",
                    phase.typist.sent, phase.typist.phase_label
                );
                if let Some(evidence) =
                    caret_evidence(&phase.before, &phase.after, phase.typist.sent)
                {
                    let _ = writeln!(out, "{evidence}");
                }
            }
            if let Some(detected) = phase.click_detected {
                let _ = writeln!(out, "Click detected on HUD: {detected}");
            }
            if let Some(check) = &phase.instrument_check {
                let _ = writeln!(out, "Instrument check attempts:");
                for attempt in &check.attempts {
                    if attempt.skipped {
                        let _ = writeln!(
                            out,
                            "  attempt {}: skipped (could not restore a different foreground \
                             window)",
                            attempt.attempt
                        );
                    } else {
                        let _ = writeln!(
                            out,
                            "  attempt {}: hook={} sampler={}",
                            attempt.attempt, attempt.hook_events, attempt.sampler_events
                        );
                    }
                }
                let _ = writeln!(
                    out,
                    "hook alive: {} (first fired on attempt {:?}), sampler alive: {} (first \
                     fired on attempt {:?})",
                    check.hook_alive,
                    check.hook_first_attempt,
                    check.sampler_alive,
                    check.sampler_first_attempt
                );
            }
            if let Some(quiescence) = &phase.quiescence {
                match quiescence.settled_after {
                    Some(d) => {
                        let _ = writeln!(out, "Quiescence: settled after {}ms", d.as_millis());
                    }
                    None => {
                        let _ = writeln!(
                            out,
                            "Quiescence: TIMED OUT after {}s (foreground did not settle; the \
                             next phase's counts may include leftover transitions from this one)",
                            QUIESCENCE_TIMEOUT.as_secs()
                        );
                    }
                }
            }
            let _ = writeln!(out);
        }

        let total_typed: usize = phases.iter().map(|p| p.typist.sent).sum();
        let _ = writeln!(out, "--- Typist summary ---");
        let _ = writeln!(
            out,
            "Total characters sent across the whole run: {total_typed}"
        );
        for phase in phases {
            if phase.typist.sent > 0 {
                let _ = writeln!(
                    out,
                    "  characters 1-{} sent during phase {} ({})",
                    phase.typist.sent, phase.id, phase.label
                );
            }
        }
        if total_typed == 0 {
            let _ = writeln!(out, "  (typist did not run in any phase)");
        }
        let _ = writeln!(out);

        // --- Judgement -----------------------------------------------------
        // The "events" column below is the SAMPLER count — it is
        // authoritative for pass/fail (see `ForegroundSampler`'s doc
        // comment for the real-hardware evidence of hook drops that
        // motivated this). The hook count is still shown, and flagged with
        // a disagreement note above when it differs, purely as a
        // cross-check.
        //
        // "Activation unchanged" is `FocusSnapshot::activation_unchanged`
        // (foreground/active/focus/caret-owner HWNDs, title, pid) — it
        // deliberately does NOT include `gui_caret_rect`. See that method's
        // doc comment for why: rcCaret is expected to move during P2 (the
        // probe's own typist is running), so it is reported separately as
        // "Caret evidence" above, not folded into this verdict.
        let _ = writeln!(out, "--- Judgement ---");
        let _ = writeln!(
            out,
            "{:<24} | hook | sampler | activation unchanged | verdict",
            "Phase"
        );

        // KO-1 (P1-P3, the design's core non-activation requirement) and
        // C7 (P4, clicking the HUD) are scored and reported SEPARATELY —
        // see the "Instrument health / KO-1 / C7" summary and the overall
        // verdict below. They mean very different things for the design:
        // a KO-1 failure means the design itself doesn't hold and needs
        // rework, while a C7 failure is a secondary condition the intended
        // design doesn't even exercise (the HUD is display-only; input
        // arrives over HID, never by clicking it). Collapsing them into
        // one FAIL would erase exactly that distinction.
        //
        // P4b (the post-click spot-check, run last) is neither: it is
        // purely informational, excluded from both KO-1 and C7, and only
        // feeds the "(hook may be degraded after the click — see P4b)"
        // annotation on the C7 line below.
        let mut ko1_all_pass = true;
        let mut c7_verdict: Option<&'static str> = None;
        let mut p4b_hook_events: Option<usize> = None;

        for phase in phases {
            let hook_count = phase.hook_events.len();
            let sampler_count = phase.sampler_events.len();
            let unchanged = phase.before.activation_unchanged(&phase.after);
            let is_p3b = phase.id == "P3b-instrument-postcheck";
            let is_p0b = phase.id == "P0b-instrument-precheck";
            let is_p4 = phase.id == "P4-click";
            let is_p4b = phase.id == "P4b-instrument-final";

            let verdict = if is_p3b || is_p0b {
                match &phase.instrument_check {
                    Some(check) if check.hook_alive && check.sampler_alive => {
                        "PASS (both instruments confirmed alive across retries)"
                    }
                    _ => "FAIL (see instrument health below)",
                }
            } else if is_p4b {
                // Informational only — see the comment above this loop.
                "INFO (not scored — see C7 annotation above)"
            } else if is_p4 && phase.click_detected != Some(true) {
                // A click that was never detected is not evidence of
                // anything — it is the same hole the instrument-health
                // VOID rule closes for a dead instrument: measuring
                // nothing must never print as a PASS. See the
                // "C7 UNVERIFIED" branch of the overall verdict.
                "SKIPPED (no click detected)"
            } else if sampler_count == 0 && hook_count == 0 && unchanged {
                // Both instruments must be silent — they fail in opposite
                // directions, so neither one alone is a sufficient gate:
                //
                //   - the hook UNDER-reports. Real hardware proved it more
                //     than once: it missed the return-to-editor transition
                //     between P4 and the old postcheck, and it goes silent
                //     for the rest of the run after P4's click specifically
                //     (see the module doc) — both of which before/after
                //     snapshots prove happened. That is why the sampler
                //     exists, and why P4b/P3b's phase ordering exist.
                //   - the hook does not OVER-report. It never invents a
                //     transition that did not occur, so a single hook event
                //     is positive evidence that the foreground really moved
                //     — a real run's P4 proved this too: the hook caught a
                //     sub-20ms HUD activation on a click that the sampler's
                //     20ms poll slept straight through.
                //   - the sampler polls at 20ms, so it cannot miss a
                //     sustained transition, but a blip shorter than one poll
                //     interval can pass between two samples unseen — exactly
                //     the case the hook does catch.
                //
                // Scoring on the sampler alone would let that sub-20ms
                // activation be reported as a PASS. Requiring both to be
                // zero is strictly stronger than either alone. Do not
                // change this to an OR, and do not drop the hook check.
                "PASS"
            } else {
                "FAIL"
            };

            if is_p4 {
                c7_verdict = Some(verdict);
            } else if is_p4b {
                p4b_hook_events = Some(hook_count);
            } else if !is_p3b && !is_p0b && verdict != "PASS" {
                ko1_all_pass = false;
            }

            let _ = writeln!(
                out,
                "{:<24} | {:<4} | {:<7} | {:<19} | {}",
                phase.id, hook_count, sampler_count, unchanged, verdict
            );
        }
        let _ = writeln!(out);

        let p0b_check = phases
            .iter()
            .find(|p| p.id == "P0b-instrument-precheck")
            .and_then(|p| p.instrument_check.as_ref());
        let p3b_check = phases
            .iter()
            .find(|p| p.id == "P3b-instrument-postcheck")
            .and_then(|p| p.instrument_check.as_ref());
        let instrument_problems = instrument_health_problems(p0b_check, p3b_check);
        let instrument_health = instrument_health_line(p0b_check, p3b_check, &instrument_problems);

        // If P0b/P3b's trailing `wait_for_quiescence` timed out, their own
        // "back to editor" cleanup transition may not have been fully
        // absorbed into that phase's window — flag it on the KO-1 line so
        // a P1-P3 FAIL that coincides with a quiescence timeout isn't read
        // as an unqualified "the design failed" without this caveat.
        let mut ko1_quiescence_warnings: Vec<String> = Vec::new();
        for (phase_name, phase_id) in [
            ("P0b", "P0b-instrument-precheck"),
            ("P3b", "P3b-instrument-postcheck"),
        ] {
            let timed_out = phases
                .iter()
                .find(|p| p.id == phase_id)
                .and_then(|p| p.quiescence.as_ref())
                .is_some_and(|q| q.settled_after.is_none());
            if timed_out {
                ko1_quiescence_warnings.push(format!(
                    "warning: foreground did not settle after {phase_name}; a leaked \
                     transition may be counted here"
                ));
            }
        }

        let ko1_line = if ko1_all_pass { "PASS" } else { "FAIL" };
        let ko1_line = if ko1_quiescence_warnings.is_empty() {
            ko1_line.to_string()
        } else {
            format!("{ko1_line} ({})", ko1_quiescence_warnings.join("; "))
        };
        // C7(P4)'s own PASS/FAIL is unaffected by P4b — P4b only adds a
        // transparency note when the hook (specifically) looks dead right
        // after the click, since that's the exact failure mode the module
        // doc's "Runs 4 and 5" note documents.
        let c7_line = match (c7_verdict.unwrap_or("FAIL"), p4b_hook_events) {
            (verdict, Some(0)) => {
                format!("{verdict} (hook may be degraded after the click — see P4b)")
            }
            (verdict, _) => verdict.to_string(),
        };
        let _ = writeln!(out, "Instrument health : {instrument_health}");
        let _ = writeln!(out, "KO-1 core (P1-P3) : {ko1_line}");
        let _ = writeln!(out, "C7 click   (P4)   : {c7_line}");
        let _ = writeln!(out);

        // Overall judgement priority:
        //   1. Instrument health failure -> VOID, unconditionally first —
        //      unchanged from before: an unconfirmed instrument makes
        //      every other line here untrustworthy.
        //   2. KO-1 FAIL -> FAIL (KO-1 core): the design's core
        //      non-activation requirement itself did not hold.
        //   3. C7 SKIPPED -> PASS (KO-1 core) / C7 UNVERIFIED.
        //   4. KO-1 PASS, C7 FAIL -> PASS (KO-1 core); C7 FAILED — recorded
        //      as a known, non-blocking condition (see the Note below),
        //      not folded into a single FAIL.
        //   5. everything PASS -> PASS (KO-1 core and C7).
        let c7_failed = matches!(
            c7_verdict,
            Some(v) if v != "PASS" && v != "SKIPPED (no click detected)"
        );

        let overall = if !instrument_problems.is_empty() {
            format!(
                "VOID ({}: the measurement instruments cannot be confirmed alive throughout \
                 the run, so a PASS on P1-P4 cannot be trusted and is not reported as one)",
                instrument_problems.join("; ")
            )
        } else if !ko1_all_pass {
            "FAIL (KO-1 core): at least one of P1-P3 recorded a foreground-change event on the \
             sampler or the hook, or its before/after activation state differs — the design's \
             core non-activating requirement did not hold"
                .to_string()
        } else if c7_verdict == Some("SKIPPED (no click detected)") {
            "PASS (KO-1 core) / C7 UNVERIFIED (no click detected)".to_string()
        } else if c7_failed {
            "PASS (KO-1 core); C7 FAILED (clicking the HUD briefly activates it)".to_string()
        } else {
            "PASS (KO-1 core and C7)".to_string()
        };
        let _ = writeln!(out, "Overall judgement: {overall}");
        if c7_failed && ko1_all_pass && instrument_problems.is_empty() {
            let _ = writeln!(
                out,
                "Note: the activation was shorter than the sampler's 20ms poll interval and the \
                 foreground returned to the previous window on its own (this phase's before/after \
                 snapshots are identical). The HUD is display-only in the intended design — all \
                 input arrives over HID to the Studio process, never by clicking the HUD — so this \
                 is recorded, not treated as a blocker. If it ever needs to be eliminated, the \
                 canonical fix is handling WM_MOUSEACTIVATE and returning MA_NOACTIVATE."
            );
        }
        let _ = writeln!(out);

        let _ = writeln!(out, "--- Manual verification (cannot be automated) ---");
        let _ = writeln!(
            out,
            "1. Before running this probe, open Notepad (or any plain text editor) and give it input focus."
        );
        let _ = writeln!(
            out,
            "2. Run `keylink-studio.exe --hud-focus-probe`. Phase order is P0b, P1, P2, P3, P3b, \
             P4, P4b — P4 (the click) runs LAST among the scored phases on purpose (see the \
             module doc). What to do depends on the phase (~57s worst case: 2s startup wait + \
             up to 6s P0b + 1s P1 + 20s P2 + ~4s P3 + up to 6s P3b + 20s P4 + 1s P4b):"
        );
        let _ = writeln!(
            out,
            "   - P0b (up to ~6s): the HUD briefly flashes to the foreground, then hides. If \
             your editor doesn't automatically regain focus on its own, the HUD reappears \
             asking you to click the editor, with a countdown — do that once, then leave \
             everything alone again."
        );
        let _ = writeln!(
            out,
            "   - P1-P3 (~25s): do NOT touch the mouse or keyboard."
        );
        let _ = writeln!(
            out,
            "   - P3b (up to ~6s): same as P0b — a brief HUD flash, then possibly a \"click the \
             editor to bring it back\" prompt. This can happen here too, not just at P0b; treat \
             it the same way."
        );
        let _ = writeln!(
            out,
            "   - P4 (~20s, LAST): the HUD will say \"ここをクリックしてください\" with a \
             countdown — click once anywhere on the HUD panel. It will switch to \"検出しました\" \
             once the click registers; nothing more is needed for the rest of P4."
        );
        let _ = writeln!(
            out,
            "   - P4b (final ~1s): fully automatic, no action needed. (This is an informational \
             spot-check, not scored — see \"C7 click\" below.)"
        );
        let _ = writeln!(
            out,
            "3. Count the characters actually typed into Notepad and compare against \"Total characters sent\" \
             above (all of it happens during P2)."
        );
        let _ = writeln!(
            out,
            "4. A match confirms Notepad (not the HUD) kept keyboard focus throughout P2; a mismatch means \
             some keystrokes landed elsewhere (or nowhere) and the HUD probably grabbed focus despite the \
             event log."
        );
        let _ = writeln!(
            out,
            "5. Check the P4-click row above: it must show a click was detected (not \"SKIPPED\"). If it says \
             SKIPPED, the C7 line reads UNVERIFIED regardless of how clean everything else looks — rerun and \
             make sure to click the HUD during P4."
        );
        let _ = writeln!(
            out,
            "6. Check the P0b/P3b rows, the \"Instrument health\" line, and the overall verdict: if it reads \
             VOID, one or both instruments failed to fire during this run's own self-checks (see the reason \
             listed in parentheses) and nothing else in this log should be trusted as a real measurement — \
             rerun rather than interpreting KO-1/C7."
        );
        let _ = writeln!(
            out,
            "7. Recall whether the HUD showed the Japanese text \"ここをクリックしてください（残り N秒）\" \
             during P4. If it did not — e.g. the HUD looked blank or showed a browser error page instead \
             — the probe page was not actually loading, which means P3 (and this note) measured nothing \
             regardless of what the log's event counts say. See \"HUD webview URL\" above and its \
             reliability note, and \"Dev-server content check warning\" if present."
        );
        let _ = writeln!(
            out,
            "8. \"KO-1 core (P1-P3)\" is the design's real pass/fail: it must read PASS. \"C7 click (P4)\" \
             failing (clicking the HUD briefly activates it) is recorded but does not block the design, \
             since the intended design never has anyone click the HUD — see the Note printed above when C7 \
             fails."
        );

        out
    }

    fn format_event(event: &ForegroundEvent) -> String {
        format!(
            "t={}ms hwnd=0x{:X} pid={} phase={} title={:?}",
            event.elapsed_ms, event.hwnd, event.pid, event.phase, event.title
        )
    }
}
