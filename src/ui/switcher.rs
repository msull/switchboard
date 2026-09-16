//! The Settings menu and the toasts (notice, host error) that float at
//! the top of the window. The project switcher itself is the rail.

use egui::{Context, RichText, Ui};

use super::{DrawCtx, theme};
use crate::core::{AppAction, ThemeMode, VoiceSettings};

/// The Prompt Box fields as typed in the menu, until each loses focus.
#[derive(Debug, Clone, Default)]
pub struct VoiceDraft {
    pub trigger: String,
    pub model: String,
    pub key: String,
}

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
            let mut open_terminal = settings.open_terminal_on_launch;
            if ui
                .checkbox(&mut open_terminal, "Open the terminal when an agent starts")
                .on_hover_text(
                    "Off, a started or resumed agent runs in its pane and the window opens only from Open",
                )
                .changed()
            {
                cx.dispatch(AppAction::SetOpenTerminalOnLaunch(open_terminal));
                ui.close();
            }
            ui.add_space(6.0);
            prompt_box_settings(cx, ui, settings);
        },
    );
}

/// The embedded Prompt Box: on or off, and what it needs. Its state is
/// Switchboard's own (this file and the Keychain), never the standalone
/// app's.
fn prompt_box_settings(cx: &mut DrawCtx<'_>, ui: &mut Ui, settings: &crate::core::Settings) {
    let p = theme::palette(ui);
    theme::kicker(ui, "Prompt Box", p.n600);
    let mut on = settings.prompt_box;
    if ui
        .checkbox(&mut on, "Prompt Box editor for agent sessions")
        .on_hover_text("Voice dictation, AI clean-up, and Save in the message box")
        .changed()
    {
        cx.dispatch(AppAction::SetPromptBox(on));
        ui.close();
    }
    let voice = settings.voice.clone();
    let mut captions = voice.captions;
    if ui
        .checkbox(&mut captions, "Captions while listening")
        .changed()
    {
        cx.dispatch(AppAction::SetVoiceSettings(VoiceSettings {
            captions,
            ..voice.clone()
        }));
    }
    let draft = cx.state.voice_draft.get_or_insert_with(|| VoiceDraft {
        trigger: voice.trigger.clone(),
        model: voice.openai_model.clone(),
        key: String::new(),
    });
    let trigger = ui.add(
        egui::TextEdit::singleline(&mut draft.trigger)
            .hint_text("Trigger word (blank: Zevro)")
            .desired_width(220.0),
    );
    let model = ui.add(
        egui::TextEdit::singleline(&mut draft.model)
            .hint_text("OpenAI model (blank: Prompt Box's default)")
            .desired_width(220.0),
    );
    let key = ui.add(
        egui::TextEdit::singleline(&mut draft.key)
            .password(true)
            .hint_text("OpenAI API key (kept in the Keychain)")
            .desired_width(220.0),
    );
    // Read what changed while the draft is borrowed, act once it is not.
    let words = ((trigger.lost_focus() || model.lost_focus())
        && (draft.trigger.trim() != voice.trigger || draft.model.trim() != voice.openai_model))
        .then(|| {
            (
                draft.trigger.trim().to_owned(),
                draft.model.trim().to_owned(),
            )
        });
    let new_key = (key.lost_focus() && !draft.key.trim().is_empty())
        .then(|| std::mem::take(&mut draft.key).trim().to_owned());
    if let Some((trigger, openai_model)) = words {
        cx.dispatch(AppAction::SetVoiceSettings(VoiceSettings {
            trigger,
            openai_model,
            captions: voice.captions,
        }));
    }
    if let Some(value) = new_key {
        cx.state.prompt_boxes.set_key(Some(value.clone()));
        cx.dispatch(AppAction::StoreVoiceKey(value));
    }
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
