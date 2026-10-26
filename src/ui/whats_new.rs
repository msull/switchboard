//! The What's new dialog: a newer release's notes when one is on offer,
//! then the changelog this binary was built with, newest first. It only
//! reads the core; the one thing it changes is whether it is open.

use egui::{Context, Ui};

use super::{DrawCtx, PAD, theme};
use crate::core::changelog::{Group, NotesTag, NotesView};

/// The dialog's width: an entry reads as a short paragraph.
const DIALOG_WIDTH: f32 = 560.0;
/// The scrolled list's height, so a long changelog leaves Close on screen.
const LIST_HEIGHT: f32 = 420.0;

/// Draw the dialog while `whats_new_dialog` is set; Escape and Close clear it.
pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    if !cx.state.whats_new_dialog {
        return;
    }
    let mut keep = !ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    super::dialogs::dialog(ctx, "What's new", |ui| {
        let screen = ctx.content_rect().width();
        ui.set_max_width((screen - 80.0).clamp(320.0, DIALOG_WIDTH));
        egui::ScrollArea::vertical()
            .max_height(LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| sections(cx, ui));
        ui.horizontal(|ui| {
            if theme::ghost_muted(ui, "Close").clicked() {
                keep = false;
            }
        });
    });
    cx.state.whats_new_dialog = keep;
}

fn sections(cx: &DrawCtx<'_>, ui: &mut Ui) {
    for section in cx.core.whats_new() {
        heading(ui, &section);
        groups(ui, &section.groups);
        if section.tag.is_some_and(NotesTag::is_found) {
            ui.label(theme::meta_text(
                ui,
                "Earlier changes since your version appear here after you update.",
            ));
        }
        ui.add_space(PAD);
    }
}

/// `0.7.3 · 2026-10-26` with its tag beside it.
fn heading(ui: &mut Ui, section: &NotesView) {
    let p = theme::palette(ui);
    let text = match &section.date {
        Some(date) => format!("{} · {date}", section.version),
        None => section.version.clone(),
    };
    ui.horizontal(|ui| {
        ui.label(theme::strong_text(text));
        if let Some(tag) = section.tag {
            theme::kicker(ui, tag.label(), p.accent_text);
        }
    });
}

/// Each group's title as a kicker and its entries as wrapped lines,
/// bulleted under a title. Entries are plain text: a backtick shows.
fn groups(ui: &mut Ui, groups: &[Group]) {
    let p = theme::palette(ui);
    for group in groups {
        if group.title.is_empty() {
            for item in &group.items {
                ui.add(egui::Label::new(item).wrap());
            }
        } else {
            theme::kicker(ui, &group.title, p.n600);
            for item in &group.items {
                ui.add(egui::Label::new(format!("•  {item}")).wrap());
            }
        }
    }
}
