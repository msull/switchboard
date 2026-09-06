//! egui drawing. Reads state from the core, turns widget events into
//! actions, and nothing else. Implemented by the UI work item.

use egui::Ui;

use crate::app::SwitchboardApp;

pub fn draw(app: &mut SwitchboardApp, ui: &mut Ui) {
    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Switchboard");
        ui.label(format!("{} project(s)", app.core().workspaces().len()));
    });
}
