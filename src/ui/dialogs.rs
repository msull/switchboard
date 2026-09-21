//! Small modal dialogs: add a project, start a session. Each is a draft
//! struct in `UiState` that exists only while its window is open.

use std::path::PathBuf;

use egui::{Context, RichText, Ui};

use super::{DrawCtx, GAP, theme};
use crate::core::{AgentKind, AppAction, Launch, Project, ProjectId, SessionKind};

#[derive(Debug, Default, Clone)]
pub struct AddProjectDraft {
    pub name: String,
    pub root: String,
}

#[derive(Debug, Clone)]
pub struct NewSessionDraft {
    pub project: ProjectId,
    pub name: String,
    pub kind: SessionKind,
    pub cwd: String,
    pub command: String,
    /// Output patterns for a command, one per line or comma-separated.
    pub outputs: String,
}

impl NewSessionDraft {
    #[must_use]
    pub fn new(project: &Project) -> Self {
        Self {
            project: project.id,
            name: String::new(),
            kind: SessionKind::Shell,
            cwd: project.root.display().to_string(),
            command: String::new(),
            outputs: String::new(),
        }
    }

    /// The action this draft becomes. Agents get `Launch::Shell` too: the
    /// core composes their argv from the kind, so the UI never guesses it.
    #[must_use]
    pub fn into_action(self, shell: String) -> AppAction {
        let launch = match self.kind {
            SessionKind::Shell | SessionKind::Agent(_) => Launch::Shell,
            SessionKind::Command | SessionKind::Service => Launch::Command {
                command: self.command.trim().to_string(),
                shell,
            },
        };
        let outputs = if self.kind == SessionKind::Command {
            split_patterns(&self.outputs)
        } else {
            Vec::new()
        };
        AppAction::NewSession {
            project: self.project,
            name: self.name.trim().to_string(),
            kind: self.kind,
            cwd: PathBuf::from(self.cwd.trim()),
            launch,
            outputs,
        }
    }
}

/// Patterns as typed: separated by newlines or commas, blanks dropped.
#[must_use]
pub fn split_patterns(text: &str) -> Vec<String> {
    text.split(['\n', ','])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

impl NewSessionDraft {
    /// Kept for symmetry with `into_action`; nothing else yet.
    #[must_use]
    pub fn is_command(&self) -> bool {
        self.kind == SessionKind::Command
    }
}

pub fn show(cx: &mut DrawCtx<'_>, ctx: &Context) {
    add_project(cx, ctx);
    new_session(cx, ctx);
    raw_message(cx, ctx);
    message_links(cx, ctx);
    delete_set(cx, ctx);
}

/// Confirm dropping a working set. Its cards are only references, so
/// nothing else is lost.
fn delete_set(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(id) = cx.state.delete_set else {
        return;
    };
    let Some(name) = cx.core.working_set(id).map(|s| s.name.clone()) else {
        cx.state.delete_set = None;
        return;
    };
    let mut done = false;
    dialog(ctx, "Delete working set", |ui| {
        ui.label(format!(
            "Delete \"{name}\"? Its sessions and files stay where they are."
        ));
        let (confirmed, cancelled) = dialog_actions(ui, "Delete set", true);
        if confirmed {
            cx.dispatch(AppAction::DeleteWorkingSet(id));
            done = true;
        }
        if cancelled {
            done = true;
        }
    });
    if done || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cx.state.delete_set = None;
    }
}

/// One message as the transcript holds it, in a monospace box that
/// scrolls, with nothing rendered: the fallback when Markdown goes
/// wrong. The text is shown read-only and selectable.
/// The two ways the message dialog shows a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageView {
    /// The text as it is, in monospace.
    Raw,
    /// Markdown drawn, as in the session view.
    #[default]
    Rendered,
}

fn raw_message(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(text) = cx.state.raw_message.clone() else {
        return;
    };
    let mut close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut view = cx.state.message_view;
    let screen = ctx.content_rect();
    // Most of the window, so a long answer reads like a page.
    let width = (screen.width() * 0.72).clamp(320.0, 1080.0);
    let height = (screen.height() - 180.0).max(160.0);
    dialog(ctx, "Full message", |ui| {
        let p = theme::palette(ui);
        ui.set_width(width);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (label, mode) in [
                ("Raw", MessageView::Raw),
                ("Rendered", MessageView::Rendered),
            ] {
                let button = if view == mode {
                    theme::ghost(ui, label)
                } else {
                    theme::ghost_muted(ui, label)
                };
                if button.clicked() {
                    view = mode;
                }
            }
        });
        egui::ScrollArea::both()
            .id_salt(("message-dialog", view == MessageView::Rendered))
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if view == MessageView::Rendered {
                    // Prose wraps at the dialog; a table wider than it
                    // scrolls sideways, as in the session view.
                    ui.set_max_width(width);
                    egui::Frame::new()
                        .fill(p.surface)
                        .inner_margin(egui::Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_width(width - 32.0);
                            super::document::markdown_style(ui);
                            super::markdown::show(ui, &mut cx.state.markdown, &text);
                        });
                } else {
                    let mut shown = text.as_str();
                    ui.add(
                        egui::TextEdit::multiline(&mut shown)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(width)
                            .frame(egui::Frame::new().fill(p.surface).inner_margin(8)),
                    );
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if theme::ghost_muted(ui, "Close").clicked() {
                close = true;
            }
            if theme::secondary(ui, "Copy").clicked() {
                ui.ctx().copy_text(text.clone());
            }
        });
    });
    cx.state.message_view = view;
    if close {
        cx.state.raw_message = None;
    }
}

/// The web links of one message as a list to click, with the dialog's
/// scroll for a long one; each opens in the browser.
fn message_links(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(links) = cx.state.message_links.clone() else {
        return;
    };
    let mut close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let screen = ctx.content_rect();
    let width = (screen.width() * 0.5).clamp(320.0, 720.0);
    let height = (screen.height() - 220.0).max(120.0);
    dialog(ctx, "Links in message", |ui| {
        ui.set_width(width);
        egui::ScrollArea::vertical()
            .id_salt("message-links")
            .max_height(height)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for url in &links {
                    ui.horizontal(|ui| {
                        ui.hyperlink(url);
                        if theme::ghost_muted(ui, "Copy")
                            .on_hover_text("Copy this link")
                            .clicked()
                        {
                            ui.ctx().copy_text(url.clone());
                        }
                    });
                }
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if theme::ghost_muted(ui, "Close").clicked() {
                close = true;
            }
            if theme::secondary(ui, "Copy all").clicked() {
                ui.ctx().copy_text(links.join("\n"));
            }
        });
    });
    if close {
        cx.state.message_links = None;
    }
}

/// The `http` and `https` URLs in `text`, in order of first appearance,
/// each once. A URL runs to whitespace or a quote or angle bracket, and
/// loses the punctuation prose or Markdown hangs on its end (a period,
/// a comma, the `)` of a `[text](url)` link) so the link still opens.
#[must_use]
pub fn web_links(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("http") {
        let candidate = &rest[start..];
        let Some(after_scheme) = candidate
            .strip_prefix("https://")
            .or_else(|| candidate.strip_prefix("http://"))
        else {
            rest = &candidate[4..];
            continue;
        };
        let end = after_scheme
            .find(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'' | '`'))
            .unwrap_or(after_scheme.len());
        let mut url = &candidate[..candidate.len() - after_scheme.len() + end];
        // Trailing punctuation belongs to the sentence, not the link,
        // except a `)` that closes a `(` inside the URL (Wikipedia).
        loop {
            let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?', ']', '}', '*']);
            let unbalanced = trimmed.ends_with(')')
                && trimmed.matches('(').count() < trimmed.matches(')').count();
            let next = if unbalanced {
                &trimmed[..trimmed.len() - 1]
            } else {
                trimmed
            };
            if next.len() == url.len() {
                break;
            }
            url = next;
        }
        if url.len() > "https://".len() && !found.iter().any(|f| f == url) {
            found.push(url.to_owned());
        }
        rest = &candidate[candidate.len() - after_scheme.len() + end..];
    }
    found
}

/// A single-line text field under its label, which tests (and screen
/// readers) find it by.
pub(super) fn field(ui: &mut Ui, label: &str, value: &mut String) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing.y = 4.0;
    let id = ui
        .label(RichText::new(label).text_style(theme::meta()).color(p.n600))
        .id;
    ui.add(
        egui::TextEdit::singleline(value)
            .background_color(p.bg)
            .margin(egui::Margin::symmetric(10, 7))
            .desired_width(360.0),
    )
    .labelled_by(id);
    ui.add_space(6.0);
}

/// A dialog window: no native title bar, the title as a heading, then
/// `body`. Surface fill and the large shadow come from the theme.
pub(super) fn dialog(ctx: &Context, title: &str, body: impl FnOnce(&mut Ui)) {
    scrim(ctx);
    // The window gets no title of its own: the heading below is the one
    // place the title appears, on screen and in the accessibility tree.
    egui::Window::new("")
        .id(egui::Id::new(("dialog", title)))
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
            ui.label(RichText::new(title).text_style(theme::brand()));
            ui.add_space(8.0);
            body(ui);
        });
}

/// A translucent sheet over the page under a dialog, so the dialog
/// reads as the thing in front. Drawn as an area below the window and
/// not interactable, so it takes no clicks itself.
fn scrim(ctx: &Context) {
    let dark = ctx.theme() == egui::Theme::Dark;
    let color = if dark {
        egui::Color32::from_black_alpha(120)
    } else {
        egui::Color32::from_black_alpha(60)
    };
    egui::Area::new(egui::Id::new("dialog-scrim"))
        .order(egui::Order::Middle)
        .interactable(false)
        .fixed_pos(egui::Pos2::ZERO)
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect();
            ui.painter().rect_filled(screen, 0.0, color);
            ui.allocate_rect(screen, egui::Sense::hover());
        });
}

/// The action row of a dialog: Cancel, then the primary action last.
/// Drawn left to right: a right-to-left row inside an auto-sized window
/// never settles on a width.
pub(super) fn dialog_actions(ui: &mut Ui, primary: &str, ready: bool) -> (bool, bool) {
    let mut confirmed = false;
    let mut cancelled = false;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if theme::ghost_muted(ui, "Cancel").clicked() {
            cancelled = true;
        }
        if ui
            .add_enabled_ui(ready, |ui| theme::primary(ui, primary))
            .inner
            .clicked()
        {
            confirmed = true;
        }
    });
    (confirmed, cancelled)
}

fn add_project(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.add_project.take() else {
        return;
    };
    let mut keep = true;
    dialog(ctx, "Add a project", |ui| {
        field(ui, "Name", &mut draft.name);
        field(ui, "Root path", &mut draft.root);
        let ready = !draft.name.trim().is_empty() && !draft.root.trim().is_empty();
        let (add, cancel) = dialog_actions(ui, "Add", ready);
        if add {
            cx.dispatch(AppAction::AddProject {
                name: draft.name.trim().to_string(),
                root: PathBuf::from(draft.root.trim()),
            });
            keep = false;
        }
        if cancel {
            keep = false;
        }
    });
    if keep {
        cx.state.add_project = Some(draft);
    }
}

fn new_session(cx: &mut DrawCtx<'_>, ctx: &Context) {
    let Some(mut draft) = cx.state.new_session.take() else {
        return;
    };
    let mut keep = true;
    dialog(ctx, "Create a session", |ui| {
        field(ui, "Name", &mut draft.name);
        kind_radios(ui, &mut draft.kind);
        field(ui, "Directory", &mut draft.cwd);
        let needs_command = matches!(draft.kind, SessionKind::Command | SessionKind::Service);
        if needs_command {
            field(ui, "Command line", &mut draft.command);
        }
        if draft.is_command() {
            field(ui, "Output files", &mut draft.outputs);
            ui.label(theme::meta_text(
                ui,
                "Patterns relative to the directory, comma-separated: reports/*.pdf",
            ));
        }
        let ready = !draft.name.trim().is_empty()
            && !draft.cwd.trim().is_empty()
            && (!needs_command || !draft.command.trim().is_empty());
        let (create, cancel) = dialog_actions(ui, "Create", ready);
        if create {
            let action = draft.clone().into_action(super::session::login_shell());
            cx.dispatch(action);
            keep = false;
        }
        if cancel {
            keep = false;
        }
    });
    if keep {
        cx.state.new_session = Some(draft);
    }
}

fn kind_radios(ui: &mut Ui, kind: &mut SessionKind) {
    let p = theme::palette(ui);
    ui.label(
        RichText::new("Kind")
            .text_style(theme::meta())
            .color(p.n600),
    );
    ui.horizontal(|ui| {
        ui.radio_value(kind, SessionKind::Shell, "Shell");
        ui.radio_value(
            kind,
            SessionKind::Agent(AgentKind::ClaudeCode),
            "Claude Code",
        );
        ui.radio_value(kind, SessionKind::Agent(AgentKind::Codex), "Codex");
        ui.radio_value(kind, SessionKind::Command, "Command");
        ui.radio_value(kind, SessionKind::Service, "Service");
    });
    ui.add_space(6.0);
}

#[cfg(test)]
mod tests {
    use super::web_links;

    #[test]
    fn web_links_finds_each_url_once_in_order_without_trailing_punctuation() {
        let text = "See https://example.com/a. Also [docs](https://docs.rs/egui), \
                    then <http://old.example.org/path?q=1&r=2>, and https://example.com/a again. \
                    Not a link: httpsomething, ftp://x.y. Wikipedia keeps its paren: \
                    https://en.wikipedia.org/wiki/Rust_(programming_language).";
        assert_eq!(
            web_links(text),
            vec![
                "https://example.com/a",
                "https://docs.rs/egui",
                "http://old.example.org/path?q=1&r=2",
                "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            ]
        );
    }

    #[test]
    fn web_links_ignores_a_bare_scheme_and_quotes() {
        assert!(web_links("https:// and http:// alone").is_empty());
        assert_eq!(
            web_links("url=\"https://a.b/c\" 'https://d.e/f'"),
            vec!["https://a.b/c", "https://d.e/f"]
        );
    }
}
