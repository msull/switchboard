//! The state machine. `AppCore` holds plain data; `dispatch` applies one
//! action and returns the side effects the shell should perform. Because
//! time is passed in explicitly, tests never sleep and never depend on the
//! wall clock.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long a toast stays visible.
const TOAST_TTL: Duration = Duration::from_secs(3);

/// The current time, as seen by the core. `mono` is for durations and
/// expiry; `wall` is for anything shown to the user or persisted.
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    pub mono: Duration,
    pub wall: SystemTime,
}

impl Clock {
    /// A clock for tests: `mono_ms` milliseconds since some origin.
    #[must_use]
    pub fn at(mono_ms: u64) -> Self {
        Self {
            mono: Duration::from_millis(mono_ms),
            wall: UNIX_EPOCH + Duration::from_secs(1_700_000_000 + mono_ms / 1000),
        }
    }
}

/// A transient message shown at the bottom of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub text: String,
    pub is_error: bool,
    pub expires_at: Duration,
}

/// Everything that can happen: user input, timer ticks, and the results of
/// effects the shell ran on the core's behalf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppAction {
    NameChanged(String),
    Greet,
    CopyGreeting,
    /// Reported by the shell after it ran `Effect::WriteClipboard`.
    ClipboardWriteFinished(Result<(), String>),
    /// A frame passed; lets timed state (toasts) expire.
    Tick,
}

/// Work the core wants done but cannot do itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    WriteClipboard(String),
}

/// Top-level application state. Plain data and pure methods only.
#[derive(Debug, Default)]
pub struct AppCore {
    name: String,
    greet_count: u32,
    toast: Option<Toast>,
}

impl AppCore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The single entry point. Applies `action` at time `now` and returns
    /// the effects the shell must run.
    pub fn dispatch(&mut self, action: AppAction, now: Clock) -> Vec<Effect> {
        let mut effects = Vec::new();
        self.expire_toast(now.mono);
        match action {
            AppAction::NameChanged(name) => self.name = name,
            AppAction::Greet => self.greet_count += 1,
            AppAction::CopyGreeting => effects.push(Effect::WriteClipboard(self.greeting())),
            AppAction::ClipboardWriteFinished(Ok(())) => {
                self.show_toast("Greeting copied", false, now);
            }
            AppAction::ClipboardWriteFinished(Err(e)) => {
                self.show_toast(format!("Copy failed: {e}"), true, now);
            }
            AppAction::Tick => {}
        }
        effects
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The greeting for the current name.
    #[must_use]
    pub fn greeting(&self) -> String {
        let name = self.name.trim();
        if name.is_empty() {
            "Hello, World!".to_owned()
        } else {
            format!("Hello, {name}!")
        }
    }

    #[must_use]
    pub fn greet_count(&self) -> u32 {
        self.greet_count
    }

    #[must_use]
    pub fn toast(&self) -> Option<&Toast> {
        self.toast.as_ref()
    }

    fn show_toast(&mut self, text: impl Into<String>, is_error: bool, now: Clock) {
        self.toast = Some(Toast {
            text: text.into(),
            is_error,
            expires_at: now.mono + TOAST_TTL,
        });
    }

    fn expire_toast(&mut self, now: Duration) {
        if self.toast.as_ref().is_some_and(|t| now >= t.expires_at) {
            self.toast = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greeting_defaults_to_world() {
        assert_eq!(AppCore::new().greeting(), "Hello, World!");
    }

    #[test]
    fn greeting_uses_trimmed_name() {
        let mut core = AppCore::new();
        core.dispatch(AppAction::NameChanged("  Sully ".into()), Clock::at(0));
        assert_eq!(core.greeting(), "Hello, Sully!");
    }

    #[test]
    fn greet_increments_count_without_effects() {
        let mut core = AppCore::new();
        assert!(core.dispatch(AppAction::Greet, Clock::at(0)).is_empty());
        core.dispatch(AppAction::Greet, Clock::at(1));
        assert_eq!(core.greet_count(), 2);
    }

    #[test]
    fn copy_requests_a_clipboard_write_and_toasts_on_success() {
        let mut core = AppCore::new();
        let effects = core.dispatch(AppAction::CopyGreeting, Clock::at(0));
        assert_eq!(effects, [Effect::WriteClipboard("Hello, World!".into())]);
        core.dispatch(AppAction::ClipboardWriteFinished(Ok(())), Clock::at(10));
        assert_eq!(
            core.toast().map(|t| t.text.as_str()),
            Some("Greeting copied")
        );
    }

    #[test]
    fn clipboard_failure_shows_an_error_toast() {
        let mut core = AppCore::new();
        core.dispatch(
            AppAction::ClipboardWriteFinished(Err("no display".into())),
            Clock::at(0),
        );
        let toast = core.toast().expect("toast");
        assert!(toast.is_error);
        assert_eq!(toast.text, "Copy failed: no display");
    }

    #[test]
    fn toast_expires_with_the_clock() {
        let mut core = AppCore::new();
        core.dispatch(AppAction::ClipboardWriteFinished(Ok(())), Clock::at(0));
        core.dispatch(AppAction::Tick, Clock::at(2_999));
        assert!(core.toast().is_some());
        core.dispatch(AppAction::Tick, Clock::at(3_000));
        assert!(core.toast().is_none());
    }
}
