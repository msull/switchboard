//! The Settings menu and the toasts (notice, host error) that float at
//! the top of the window. The project switcher itself is the rail.

use egui::{Context, RichText, Ui};

use super::{DrawCtx, theme};
use crate::core::{AppAction, ThemeMode};

/// The Settings menu: theme, editor command, environment, exclusive.
/// Compact draws only a glyph, for the narrow rail.
pub fn settings_menu(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    settings: &crate::core::Settings,
    compact: bool,
) {
    let p = theme::palette(ui);
    let label = if compact { "⚙" } else { "Settings" };
    ui.menu_button(
        RichText::new(label).text_style(theme::meta()).color(p.n700),
        |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            theme::kicker(ui, "Theme", p.n600);
            for mode in ThemeMode::ALL {
                if ui.radio(mode == settings.theme, mode.label()).clicked() {
                    cx.dispatch(AppAction::SetTheme(mode));
                    ui.close();
                }
            }
            ui.add_space(6.0);
            theme::kicker(ui, "Editor command", p.n600);
            let draft = cx
                .state
                .editor_draft
                .get_or_insert_with(|| settings.editor.clone());
            let field = ui.add(
                egui::TextEdit::singleline(draft)
                    .hint_text("code, zed, cursor (blank: system editor)")
                    .desired_width(220.0),
            );
            if field.lost_focus() {
                let editor = draft.clone();
                cx.state.editor_draft = None;
                if editor.trim() != settings.editor {
                    cx.dispatch(AppAction::SetEditor(editor));
                }
            }
            if theme::ghost(ui, "Environment…").clicked() {
                cx.state.env_dialog = Some(super::env::EnvDraft::global(cx.core, cx.services));
                ui.close();
            }
            ui.add_space(6.0);
            let mut exclusive = settings.exclusive;
            if ui
                .checkbox(&mut exclusive, "Exclusive: only the active project")
                .on_hover_text("Hides every other project while screen sharing")
                .changed()
            {
                cx.dispatch(AppAction::SetExclusive(exclusive));
                ui.close();
            }
        },
    );
}

/// Notices and the host error as toasts at the top centre: surface
/// fill, a shadow, errors led by a magenta dot.
pub fn toasts(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let notice = cx.core.notice().cloned();
    let host_error = cx.core.host_error().map(str::to_owned);
    if notice.is_none() && host_error.is_none() {
        return;
    }
    egui::Area::new(egui::Id::new("toasts"))
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            let p = theme::palette(ui);
            let mut dismiss = false;
            for (text, is_error, dismissable) in host_error
                .iter()
                .map(|e| (e.clone(), true, false))
                .chain(notice.iter().map(|n| (n.text.clone(), n.is_error, true)))
            {
                egui::Frame::new()
                    .fill(p.surface)
                    .corner_radius(2)
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(egui::Margin::symmetric(14, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if is_error {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(8.0, 8.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().circle_filled(rect.center(), 4.0, p.accent_2);
                            }
                            ui.label(RichText::new(&text).text_style(theme::meta()));
                            if dismissable && theme::ghost_muted(ui, "Dismiss").clicked() {
                                dismiss = true;
                            }
                        });
                    });
            }
            if dismiss {
                cx.dispatch(AppAction::DismissNotice);
            }
        });
}
