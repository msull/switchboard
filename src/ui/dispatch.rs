//! The Dispatch pages: every ticket with what waits on the user and a
//! console for `dispatch` commands, and one ticket in full. Everything
//! shown came through Dispatch's port as a view; a button is an action
//! the core turns into one call.

use std::path::PathBuf;

use egui::{RichText, Ui};

use super::{DrawCtx, GAP, markdown, theme};
use crate::core::{AppAction, RecordId, View};
use crate::ports::dispatch::{AttemptView, DecisionView, TicketView};

/// The console pane's height on the overview.
const CONSOLE_HEIGHT: f32 = 280.0;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    theme::kicker(ui, "Dispatch", p.n600);
    let state = cx.core.dispatch_state();
    let (connected, seen) = (state.connected, state.seen);
    let tickets = state.status.tickets.clone();
    let projects = state.status.projects.clone();
    ui.horizontal(|ui| {
        ui.add(egui::Label::new(RichText::new("Tickets").text_style(theme::h1())).truncate());
        ui.label(theme::meta_text(ui, "·"));
        let standing = match (connected, seen) {
            (true, _) => format!("runner up · {} ticket(s)", tickets.len()),
            (false, true) => "runner gone; showing its last status".to_owned(),
            (false, false) => "no runner: start one from the console".to_owned(),
        };
        ui.label(
            RichText::new(standing)
                .text_style(theme::meta())
                .color(if connected { p.n700 } else { p.accent_2_text }),
        );
    });

    egui::ScrollArea::vertical()
        .id_salt("dispatch-page")
        .show(ui, |ui| {
            let pending: Vec<DecisionView> =
                cx.core.pending_decisions().into_iter().cloned().collect();
            theme::section(ui, "Waiting on you");
            if pending.is_empty() {
                ui.label(theme::meta_text(ui, "Nothing waits on you."));
            }
            for d in &pending {
                let ticket = tickets.iter().find(|t| t.id == d.ticket).cloned();
                decision_card(cx, ui, d, ticket.as_ref(), true);
            }

            for project in &projects {
                theme::section(ui, &project.name);
                let mut listed = 0;
                for id in &project.queue {
                    if let Some(t) = tickets.iter().find(|t| &t.id == id) {
                        ticket_row(cx, ui, t);
                        listed += 1;
                    }
                }
                for t in tickets
                    .iter()
                    .filter(|t| t.project == project.name && !project.queue.contains(&t.id))
                {
                    ticket_row(cx, ui, t);
                    listed += 1;
                }
                if listed == 0 {
                    ui.label(theme::meta_text(ui, "No tickets."));
                }
            }

            theme::section(ui, "Console");
            console(cx, ui);
        });
}

/// One ticket's line: number and title as the link, the stage strip
/// and its standing after.
fn ticket_row(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if theme::ghost(ui, &title_of(t)).clicked() {
            cx.dispatch(AppAction::ShowTicket(t.id.clone()));
        }
        ui.label(theme::meta_text(ui, "·"));
        stage_strip(ui, t);
        ui.label(theme::meta_text(ui, "·"));
        let waiting = t.decisions.iter().filter(|d| d.state == "pending").count();
        let standing = standing_of(t);
        ui.label(RichText::new(standing).text_style(theme::meta()).color(
            if waiting > 0 || t.state != "active" {
                p.accent_2_text
            } else {
                p.n700
            },
        ));
    });
}

fn title_of(t: &TicketView) -> String {
    match t.number {
        Some(n) => format!("#{n} {}", t.title),
        None => t.title.clone(),
    }
}

/// `active · investigate running`, `parked: <reason>`, `2 waiting`.
fn standing_of(t: &TicketView) -> String {
    let waiting = t.decisions.iter().filter(|d| d.state == "pending").count();
    if waiting > 0 {
        return format!("{waiting} waiting on you");
    }
    match t.state.as_str() {
        "active" => t
            .attempts
            .last()
            .map_or("queued".to_owned(), |a| format!("{} {}", a.stage, a.state)),
        other => match &t.reason {
            Some(reason) => format!("{other}: {reason}"),
            None => other.to_owned(),
        },
    }
}

/// The stages in order, the current one in the text colour, the rest
/// muted; done when past the end.
fn stage_strip(ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing.x = 3.0;
    for (i, name) in t.stages.iter().enumerate() {
        if i > 0 {
            ui.label(theme::meta_text(ui, "›").color(p.n600));
        }
        let color = match i.cmp(&t.stage) {
            std::cmp::Ordering::Less => p.n600,
            std::cmp::Ordering::Equal => p.text,
            std::cmp::Ordering::Greater => p.n700,
        };
        ui.label(RichText::new(name).text_style(theme::meta()).color(color));
    }
    if t.stage >= t.stages.len() && !t.stages.is_empty() {
        ui.label(theme::meta_text(ui, "› done"));
    }
}

/// A decision with its options as buttons, the recommended one filled.
/// `with_ticket` names the ticket above the question, for the overview.
fn decision_card(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    d: &DecisionView,
    ticket: Option<&TicketView>,
    with_ticket: bool,
) {
    let p = theme::palette(ui);
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if with_ticket && let Some(t) = ticket {
                ui.horizontal(|ui| {
                    if theme::ghost(ui, &title_of(t)).clicked() {
                        cx.dispatch(AppAction::ShowTicket(t.id.clone()));
                    }
                    ui.label(theme::meta_text(ui, format!("· {} · {}", d.stage, d.name)));
                });
            } else {
                ui.label(theme::meta_text(ui, format!("{} · {}", d.stage, d.name)));
            }
            ui.add(egui::Label::new(&d.question).wrap());
            if d.state != "pending" {
                ui.label(
                    RichText::new(format!(
                        "{}: {}",
                        d.state,
                        d.answer.clone().unwrap_or_default()
                    ))
                    .text_style(theme::meta())
                    .color(p.n700),
                );
                return;
            }
            ui.horizontal_wrapped(|ui| {
                for option in &d.options {
                    let recommended = d.recommendation.as_deref() == Some(option);
                    let clicked = if recommended {
                        theme::primary(ui, option)
                    } else {
                        theme::secondary(ui, option)
                    }
                    .on_hover_text(if recommended {
                        "Dispatch suggests this"
                    } else {
                        "Answer with this"
                    })
                    .clicked();
                    if clicked {
                        cx.dispatch(AppAction::DispatchDecide {
                            ticket: d.ticket.clone(),
                            decision: d.id.clone(),
                            answer: option.clone(),
                            note: None,
                        });
                    }
                }
            });
        });
}

/// The command line and the console's pane, or the button that makes
/// the console.
fn console(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let console = cx.core.console();
    ui.horizontal(|ui| {
        ui.label(theme::mono_text(ui, "dispatch"));
        let width = ui.available_width() - 80.0;
        let response = ui.add_sized(
            [width.max(120.0), 24.0],
            egui::TextEdit::singleline(&mut cx.state.dispatch_console_draft)
                .hint_text("decisions · status · take Delta 104 · !any shell line")
                .font(egui::TextStyle::Monospace),
        );
        let submit = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if theme::primary(ui, "Run").clicked() || submit {
            let line = std::mem::take(&mut cx.state.dispatch_console_draft);
            cx.dispatch(AppAction::DispatchConsole(line));
            response.request_focus();
        }
    });
    match console.and_then(|id| cx.core.session(id).cloned()) {
        Some(record) => {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), CONSOLE_HEIGHT),
                egui::Layout::top_down(egui::Align::Min),
                |ui| super::session::terminal_body(cx, ui, &record),
            );
        }
        None => {
            ui.horizontal(|ui| {
                ui.label(theme::meta_text(
                    ui,
                    "No console yet: a shell in Dispatch's data directory, under a Dispatch project.",
                ));
                if theme::secondary(ui, "Open console").clicked() {
                    cx.dispatch(AppAction::OpenDispatchConsole);
                }
            });
        }
    }
}

/// One ticket: its stages, lanes, decisions and attempts on the left,
/// the artifact being read on the right.
pub fn ticket(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: &str) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    let Some(t) = cx.core.ticket(id).cloned() else {
        theme::kicker(ui, "Dispatch ticket", p.n600);
        ui.label("This ticket is not in Dispatch's last status.");
        if theme::ghost(ui, "Back").clicked() {
            cx.dispatch(AppAction::Back);
        }
        return;
    };
    ticket_header(cx, ui, &t);

    let left = (ui.available_width() * 0.45).max(320.0);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(left, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("ticket-left")
                    .show(ui, |ui| {
                        theme::section(ui, "Decisions");
                        if t.decisions.is_empty() {
                            ui.label(theme::meta_text(ui, "None yet."));
                        }
                        for d in t.decisions.iter().rev() {
                            decision_card(cx, ui, d, Some(&t), false);
                        }
                        theme::section(ui, "Attempts");
                        if t.attempts.is_empty() {
                            ui.label(theme::meta_text(ui, "Nothing has run yet."));
                        }
                        for a in t.attempts.iter().rev() {
                            attempt_row(cx, ui, &t, a);
                        }
                    });
            },
        );
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| artifact_column(cx, ui, &t),
        );
    });
}

/// Kicker, title with the issue link and Back, the meta line and the
/// lanes.
fn ticket_header(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    theme::kicker(ui, "Dispatch ticket", p.n600);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if theme::ghost(ui, "Back").clicked() {
                cx.dispatch(AppAction::Back);
            }
            if let Some(url) = &t.url {
                ui.hyperlink_to(RichText::new("Issue").color(p.accent_text), url);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(
                    egui::Label::new(RichText::new(title_of(t)).text_style(theme::h1())).truncate(),
                );
            });
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::meta_text(ui, &t.project));
        ui.label(theme::meta_text(ui, "·"));
        stage_strip(ui, t);
        ui.label(theme::meta_text(ui, "·"));
        ui.label(
            RichText::new(standing_of(t))
                .text_style(theme::meta())
                .color(if t.state == "active" {
                    p.n700
                } else {
                    p.accent_2_text
                }),
        );
        if let Some(tree) = &t.tree {
            ui.label(theme::meta_text(ui, "·"));
            ui.label(theme::mono_text(ui, tree.display().to_string()));
        }
    });
    if !t.lanes.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(theme::meta_text(ui, "Lanes:"));
            for lane in &t.lanes {
                let text = format!(
                    "{}{}{}",
                    lane.name,
                    if lane.chosen { "" } else { " (not chosen)" },
                    if lane.setup_done { " · set up" } else { "" }
                );
                ui.label(
                    RichText::new(text)
                        .text_style(theme::meta())
                        .color(if lane.chosen { p.text } else { p.n700 }),
                )
                .on_hover_text(format!(
                    "{} on {}",
                    lane.worktree.display(),
                    lane.branch
                ));
            }
        });
    }
}

fn attempt_row(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView, a: &AttemptView) {
    let p = theme::palette(ui);
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(theme::strong_text(format!(
                    "{} · {} · #{}",
                    a.stage, a.context, a.n
                )));
                ui.label(RichText::new(&a.state).text_style(theme::meta()).color(
                    match a.state.as_str() {
                        "failed" | "cancelled" => p.accent_2_text,
                        "complete" => p.n700,
                        _ => p.accent_text,
                    },
                ));
                if let Some(reason) = &a.reason {
                    ui.label(theme::meta_text(ui, reason));
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                if let Some(session) = a
                    .session
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(RecordId)
                    .filter(|id| cx.core.session(*id).is_some())
                    && theme::ghost(ui, "Session").clicked()
                {
                    cx.dispatch(AppAction::ShowSession(session));
                }
                if let Some(run) = a
                    .run
                    .as_deref()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                    .map(crate::core::WorkflowId)
                    .filter(|id| cx.core.workflow(*id).is_some())
                    && theme::ghost(ui, "Review").clicked()
                {
                    cx.dispatch(AppAction::ShowWorkflow(run));
                }
                for (name, path) in &a.artifacts {
                    let selected = cx.state.dispatch_artifact.as_ref() == Some(path);
                    let button = if selected {
                        theme::secondary(ui, name)
                    } else {
                        theme::ghost(ui, name)
                    };
                    if button.on_hover_text(path.display().to_string()).clicked() {
                        cx.state.dispatch_artifact = Some(path.clone());
                        cx.dispatch(AppAction::DispatchReadArtifact {
                            ticket: t.id.clone(),
                            path: path.clone(),
                        });
                    }
                }
            });
        });
}

/// The artifact chosen on the left, rendered as markdown; the issue's
/// own text until one is chosen.
fn artifact_column(cx: &mut DrawCtx<'_>, ui: &mut Ui, t: &TicketView) {
    let p = theme::palette(ui);
    let chosen: Option<PathBuf> = cx.state.dispatch_artifact.clone().filter(|path| {
        t.attempts
            .iter()
            .any(|a| a.artifacts.iter().any(|(_, p)| p == path))
    });
    let (title, text) = match &chosen {
        Some(path) => (
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("artifact")
                .to_owned(),
            cx.core.dispatch_state().artifacts.get(path).cloned(),
        ),
        None => ("Issue".to_owned(), Some(t.body.clone())),
    };
    theme::section(ui, &title);
    if let Some(path) = &chosen {
        ui.label(theme::mono_text(ui, path.display().to_string()).color(p.n700));
    }
    egui::ScrollArea::vertical()
        .id_salt("ticket-artifact")
        .show(ui, |ui| match text {
            Some(text) if text.trim().is_empty() => {
                ui.label(theme::meta_text(ui, "Empty."));
            }
            Some(text) => markdown::show(ui, &mut cx.state.markdown, &text),
            None => {
                ui.label(theme::meta_text(ui, "Reading…"));
            }
        });
}

/// Whether `view` is one of Dispatch's, for the rail.
#[must_use]
pub fn is_dispatch_view(view: &View) -> bool {
    matches!(view, View::Dispatch | View::Ticket(_))
}
