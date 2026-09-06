//! Orchestration: owns [`AppCore`] and the adapters. Maps effects to
//! adapter calls and feeds results back as actions. Rendering lives in
//! [`crate::ui`].

use std::time::{Instant, SystemTime};

use crate::core::{AppAction, AppCore, Clock, Effect};
use crate::ports::agent::AgentLauncher;
use crate::ports::events::EventSource;
use crate::ports::host::ProcessHost;
use crate::ports::opener::Opener;
use crate::ports::store::Store;

pub struct Services {
    pub store: Box<dyn Store>,
    pub host: Box<dyn ProcessHost>,
    pub events: Box<dyn EventSource>,
    pub agents: Box<dyn AgentLauncher>,
    pub opener: Box<dyn Opener>,
}

pub struct SwitchboardApp {
    core: AppCore,
    services: Services,
    started: Instant,
}

impl SwitchboardApp {
    /// Creates the app with the given adapters. Real ones are assembled in
    /// `main.rs`; tests pass fakes.
    #[must_use]
    pub fn with_services(services: Services) -> Self {
        Self {
            core: AppCore::new(),
            services,
            started: Instant::now(),
        }
    }

    #[must_use]
    pub fn core(&self) -> &AppCore {
        &self.core
    }

    /// See [`AppCore::seed`]; tests and the demo launcher only.
    pub fn core_mut_for_seeding(&mut self) -> &mut AppCore {
        &mut self.core
    }

    #[must_use]
    pub fn services(&self) -> &Services {
        &self.services
    }

    fn clock(&self) -> Clock {
        Clock {
            mono: self.started.elapsed(),
            wall: SystemTime::now(),
        }
    }

    /// The single entry point for every user, worker, or timer action.
    pub fn dispatch(&mut self, action: AppAction) {
        let now = self.clock();
        let effects = self.core.dispatch(action, now);
        for effect in effects {
            if let Some(result) = self.run_effect(effect) {
                self.dispatch(result);
            }
        }
    }

    /// Performs one effect; returns the action reporting its result, or
    /// `None` when the result arrives later from a worker.
    fn run_effect(&mut self, effect: Effect) -> Option<AppAction> {
        let _ = (&self.services, effect);
        None
    }
}

impl eframe::App for SwitchboardApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.dispatch(AppAction::Tick);
        crate::ui::draw(self, ui);
    }
}
