//! Event-driven foreground-window watcher.
//!
//! Uses `SetWinEventHook(EVENT_SYSTEM_FOREGROUND)` so layer switching reacts
//! the instant the active window changes, instead of waiting for the next poll
//! tick. The monitor loop keeps its polling timeout as a fallback; this only
//! delivers an extra wake-up via `MonitorCommand::ForegroundChanged`.

use crate::state::MonitorCommand;

#[cfg(windows)]
pub use windows_impl::ForegroundWatcher;

#[cfg(not(windows))]
pub struct ForegroundWatcher;

#[cfg(not(windows))]
impl ForegroundWatcher {
    pub fn new(_tx: std::sync::mpsc::Sender<MonitorCommand>) -> Self {
        ForegroundWatcher
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::{
        cell::RefCell,
        sync::mpsc::{self, Sender},
        thread::{self, JoinHandle},
    };

    use tracing::warn;
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK},
        UI::WindowsAndMessaging::{
            DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage,
            EVENT_SYSTEM_FOREGROUND, MSG, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_QUIT,
        },
    };

    use super::MonitorCommand;

    thread_local! {
        static SENDER: RefCell<Option<Sender<MonitorCommand>>> = const { RefCell::new(None) };
    }

    pub struct ForegroundWatcher {
        thread_id: u32,
        join: Option<JoinHandle<()>>,
    }

    impl ForegroundWatcher {
        pub fn new(tx: Sender<MonitorCommand>) -> Self {
            let (id_tx, id_rx) = mpsc::channel::<u32>();
            let join = thread::spawn(move || {
                SENDER.with(|cell| *cell.borrow_mut() = Some(tx));

                let hook = unsafe {
                    SetWinEventHook(
                        EVENT_SYSTEM_FOREGROUND,
                        EVENT_SYSTEM_FOREGROUND,
                        None,
                        Some(win_event_proc),
                        0,
                        0,
                        WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                    )
                };

                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                let _ = id_tx.send(thread_id);

                if hook.is_invalid() {
                    warn!("SetWinEventHook failed; foreground watcher disabled");
                    return;
                }

                // Standard message loop; the hook callback runs on this thread.
                let mut msg = MSG::default();
                unsafe {
                    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    let _ = UnhookWinEvent(hook);
                }
                SENDER.with(|cell| *cell.borrow_mut() = None);
            });

            let thread_id = id_rx.recv().unwrap_or(0);
            Self {
                thread_id,
                join: Some(join),
            }
        }
    }

    impl Drop for ForegroundWatcher {
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
        _hwnd: HWND,
        _id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        SENDER.with(|cell| {
            if let Some(tx) = cell.borrow().as_ref() {
                let _ = tx.send(MonitorCommand::ForegroundChanged);
            }
        });
    }
}
