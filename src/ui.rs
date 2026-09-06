//! egui drawing. Reads state from the core, turns widget events into
//! actions, and nothing else.

use egui::{RichText, Ui};

use crate::app::SwitchboardApp;
use crate::core::AppAction;

pub fn draw(app: &mut SwitchboardApp, ui: &mut Ui) {
    egui::Panel::bottom("toast_bar")
        .exact_size(24.0)
        .resizable(false)
        .show(ui, |ui| toast_bar(app, ui));

    egui::CentralPanel::default().show(ui, |ui| {
        ui.heading("Switchboard");
        ui.add_space(8.0);

        ui.horizontal(|ui| {
            let label = ui.label("Name");
            let response = ui
                .text_edit_singleline(&mut app.name_input)
                .labelled_by(label.id);
            if response.changed() {
                app.dispatch(AppAction::NameChanged(app.name_input.clone()));
            }
        });

        ui.horizontal(|ui| {
            if ui.button("Greet").clicked() {
                app.dispatch(AppAction::Greet);
            }
            if ui.button("Copy greeting").clicked() {
                app.dispatch(AppAction::CopyGreeting);
            }
        });

        ui.add_space(8.0);
        ui.label(app.core().greeting());
        ui.label(format!("Greeted {} times", app.core().greet_count()));
    });
}

fn toast_bar(app: &SwitchboardApp, ui: &mut Ui) {
    if let Some(toast) = app.core().toast() {
        let color = if toast.is_error {
            ui.visuals().error_fg_color
        } else {
            ui.visuals().strong_text_color()
        };
        ui.label(RichText::new(&toast.text).color(color));
    }
}
