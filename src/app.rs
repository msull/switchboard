//! Orchestration: owns [`AppCore`] and the port adapters. Maps effects to
//! adapter calls and feeds their results back as actions. Rendering lives
//! in [`crate::ui`].

use std::time::{Duration, Instant, SystemTime};

use crate::adapters::clipboard::SystemClipboard;
use crate::core::{AppAction, AppCore, Clock, Effect};
use crate::ports::clipboard::Clipboard;

pub struct SwitchboardApp {
    core: AppCore,
    clipboard: Box<dyn Clipboard>,
    started: Instant,
    /// Text in the name box; mirrored into the core when it changes.
    pub name_input: String,
}

impl SwitchboardApp {
    /// Creates the app with real adapters. The creation context gives
    /// access to egui settings (fonts, storage, etc.) when needed.
    #[must_use]
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self::with_services(Box::new(SystemClipboard::default()))
    }

    /// Creates the app with the given adapters. Tests pass fakes.
    #[must_use]
    pub fn with_services(clipboard: Box<dyn Clipboard>) -> Self {
        Self {
            core: AppCore::new(),
            clipboard,
            started: Instant::now(),
            name_input: String::new(),
        }
    }

    #[must_use]
    pub fn core(&self) -> &AppCore {
        &self.core
    }

    fn clock(&self) -> Clock {
        Clock {
            mono: self.started.elapsed(),
            wall: SystemTime::now(),
        }
    }

    /// The single entry point for every user or timer action.
    pub fn dispatch(&mut self, action: AppAction) {
        let now = self.clock();
        let effects = self.core.dispatch(action, now);
        for effect in effects {
            if let Some(result) = self.run_effect(effect) {
                self.dispatch(result);
            }
        }
    }

    /// Performs one effect; returns the action that reports its result, or
    /// `None` when the result arrives later (for example from a worker
    /// thread via a channel drained in `ui`). Every effect here is
    /// synchronous, hence the lint allow; drop it once one is not.
    #[allow(clippy::unnecessary_wraps)]
    fn run_effect(&mut self, effect: Effect) -> Option<AppAction> {
        match effect {
            Effect::WriteClipboard(text) => Some(AppAction::ClipboardWriteFinished(
                self.clipboard.write_text(&text),
            )),
        }
    }
}

impl eframe::App for SwitchboardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.dispatch(AppAction::Tick);
        crate::ui::draw(self, ui);
        if self.core.toast().is_some() {
            // Keep repainting so the toast disappears on time without input.
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }
}
