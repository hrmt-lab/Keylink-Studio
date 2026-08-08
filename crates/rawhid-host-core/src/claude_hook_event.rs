use std::fmt;

use serde_json::Value;

#[derive(Clone, PartialEq)]
pub struct ClaudeHookEvent {
    pub launch_id: String,
    pub hook_event_name: String,
    pub session_id: Option<String>,
    pub body: Value,
}

impl ClaudeHookEvent {
    pub fn is_priority(&self) -> bool {
        matches!(
            self.hook_event_name.as_str(),
            "SessionStart"
                | "UserPromptSubmit"
                | "Stop"
                | "StopFailure"
                | "SessionEnd"
                | "PreCompact"
                | "PostCompact"
        )
    }
}

impl fmt::Debug for ClaudeHookEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaudeHookEvent")
            .field("launch_id", &self.launch_id)
            .field("hook_event_name", &self.hook_event_name)
            .field("session_id", &self.session_id)
            .field("body", &"<redacted>")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeWrapperExited {
    pub launch_id: String,
    pub exit_code: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClaudeObserverEvent {
    Hook(ClaudeHookEvent),
    WrapperExited(ClaudeWrapperExited),
}

impl ClaudeObserverEvent {
    pub fn is_priority(&self) -> bool {
        match self {
            Self::Hook(event) => event.is_priority(),
            Self::WrapperExited(_) => true,
        }
    }
}
