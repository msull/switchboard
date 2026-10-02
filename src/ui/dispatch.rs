//! The Dispatch pages: every ticket with what waits on the user and a
//! console for `dispatch` commands, and one ticket in full. Everything
//! shown came through Dispatch's port as a view; a button is an action
//! the core turns into one call.

use std::path::PathBuf;

use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};

use super::{DrawCtx, GAP, markdown, theme};
use crate::core::{AppAction, RecordId, TicketOnly, TicketSort, View, WaitingAgent};
use crate::ports::dispatch::{AttemptView, DecisionView, ProjectView, TicketView};

/// The console pane's height on the overview.
const CONSOLE_HEIGHT: f32 = 280.0;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    let state = cx.core.dispatch_state();
    let (connected, seen) = (state.connected, state.seen);
    let tickets = state.status.tickets.clone();
    let projects = state.status.projects.clone();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            pop_out_button(cx, ui);
            let open = cx.state.dispatch_settings.is_some();
            if theme::ghost(ui, if open { "Hide settings" } else { "Settings…" })
                .on_hover_text("Where tickets' trees are cut")
                .clicked()
            {
                cx.state.dispatch_settings = (!open).then(String::new);
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                theme::kicker(ui, "Dispatch", p.n600);
            });
        });
    });
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
    project_chips(cx, ui, &projects);
    let chosen = cx.state.dispatch_project.clone();
    let shown = |name: &str| chosen.as_deref().is_none_or(|c| c == name);

    egui::ScrollArea::vertical()
        .id_salt("dispatch-page")
        .show(ui, |ui| {
            // Rarely touched, so behind the header's button rather than
            // beside the console's command line, which it looked like.
            if cx.state.dispatch_settings.is_some() {
                theme::section(ui, "Settings");
                worktrees(cx, ui, &state.status.worktrees);
            }
            let pending: Vec<DecisionView> = cx
                .core
                .pending_decisions()
                .into_iter()
                .filter(|d| {
                    tickets
                        .iter()
                        .find(|t| t.id == d.ticket)
                        .is_some_and(|t| shown(&t.project))
                })
                .cloned()
                .collect();
            let agents: Vec<WaitingAgent> = cx
                .core
                .waiting_agents()
                .into_iter()
                .filter(|a| {
                    tickets
                        .iter()
                        .find(|t| t.id == a.ticket)
                        .is_some_and(|t| shown(&t.project))
                })
                .collect();
            // A few questions are shown in full; a pile of them folds
            // behind its count, so the table is not pushed off the
            // page, and the fold is remembered for the window's life.
            let waiting = pending.len() + agents.len();
            let heading = if waiting == 0 {
                "Waiting on you".to_owned()
            } else {
                format!("Waiting on you · {waiting}")
            };
            egui::CollapsingHeader::new(RichText::new(heading).text_style(theme::kicker_style()))
                .id_salt("dispatch-waiting")
                .default_open(waiting <= 3)
                .show(ui, |ui| {
                    if waiting == 0 {
                        ui.label(theme::meta_text(ui, "Nothing waits on you."));
                    }
                    for d in &pending {
                        let ticket = tickets.iter().find(|t| t.id == d.ticket).cloned();
                        decision_card(cx, ui, d, ticket.as_ref(), true);
                    }
                    for a in &agents {
                        let ticket = tickets.iter().find(|t| t.id == a.ticket).cloned();
                        agent_card(cx, ui, a, ticket.as_ref());
                    }
                });

            theme::section(ui, "Tickets");
            project_limits(ui, projects.iter().filter(|p| shown(&p.name)));
            listing_controls(cx, ui);
            ticket_table(cx, ui);

            theme::section(ui, "Console");
            console(cx, ui);
        });
}

/// Where tickets' trees go, and a field to move them: set for new
/// tickets, or set and migrate every idle ticket's tree there.
fn worktrees(cx: &mut DrawCtx<'_>, ui: &mut Ui, root: &std::path::Path) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(theme::meta_text(ui, "Tickets' trees are cut under"));
        ui.label(theme::mono_text(ui, root.display().to_string()));
    });
    ui.horizontal(|ui| {
        ui.label("Move to");
        let width = (ui.available_width() - 220.0).max(120.0);
        let draft = cx.state.dispatch_settings.get_or_insert_default();
        ui.add_sized(
            [width, 24.0],
            egui::TextEdit::singleline(draft)
                .hint_text("a directory, for example ~/.dispatch/worktrees"),
        );
        let path = draft.trim().to_owned();
        let typed = !path.is_empty();
        ui.add_enabled_ui(typed, |ui| {
            if theme::secondary(ui, "Set")
                .on_hover_text("New tickets go here; existing trees stay where they are")
                .clicked()
            {
                cx.dispatch(AppAction::DispatchWorktrees {
                    path: Some(PathBuf::from(&path)),
                    migrate: false,
                });
            }
            if theme::secondary(ui, "Set and migrate")
                .on_hover_text("Also move every ticket's tree that nothing is running in")
                .clicked()
            {
                cx.dispatch(AppAction::DispatchWorktrees {
                    path: Some(PathBuf::from(&path)),
                    migrate: true,
                });
            }
        });
    });
}

/// One chip per project, and All; the chosen one filled.
fn project_chips(cx: &mut DrawCtx<'_>, ui: &mut Ui, projects: &[ProjectView]) {
    if projects.len() < 2 {
        cx.state.dispatch_project = None;
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let mut choice = cx.state.dispatch_project.clone();
        let all = choice.is_none();
        if (if all {
            theme::primary(ui, "All")
        } else {
            theme::secondary(ui, "All")
        })
        .clicked()
        {
            choice = None;
        }
        for project in projects {
            let on = choice.as_deref() == Some(project.name.as_str());
            if (if on {
                theme::primary(ui, &project.name)
            } else {
                theme::secondary(ui, &project.name)
            })
            .on_hover_text(format!("Only {}'s tickets and decisions", project.name))
            .clicked()
            {
                choice = Some(project.name.clone());
            }
        }
        cx.state.dispatch_project = choice;
    });
}

/// One line per project: how many slots and decisions its policy
/// allows and what holds new starts back.
fn project_limits<'a>(ui: &mut Ui, projects: impl Iterator<Item = &'a ProjectView>) {
    let p = theme::palette(ui);
    for project in projects {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(theme::strong_text(&project.name));
            ui.label(theme::meta_text(
                ui,
                format!(
                    "{} of {} slots in use · {} of {} decisions waiting",
                    project.running, project.slots, project.pending, project.waiting_on_me
                ),
            ));
            if let Some(why) = project.held() {
                ui.label(theme::meta_text(ui, "·"));
                ui.label(
                    RichText::new(format!("nothing new starts: {why}"))
                        .text_style(theme::meta())
                        .color(p.accent_2_text),
                );
            }
        });
    }
}

/// The filter row: words every row must contain somewhere, and which
/// states are listed. The project chips above narrow it too.
fn listing_controls(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let label = ui.label(theme::meta_text(ui, "Filter")).id;
        ui.add(
            egui::TextEdit::singleline(&mut cx.state.dispatch_listing.text)
                .hint_text("words from the number, title, stage or standing")
                .desired_width(260.0),
        )
        .labelled_by(label);
        if !cx.state.dispatch_listing.text.is_empty() && theme::ghost_muted(ui, "Clear").clicked() {
            cx.state.dispatch_listing.text.clear();
        }
        ui.add_space(GAP);
        for only in TicketOnly::ALL {
            let on = cx.state.dispatch_listing.only == only;
            if (if on {
                theme::primary(ui, only.label())
            } else {
                theme::secondary(ui, only.label())
            })
            .clicked()
            {
                cx.state.dispatch_listing.only = only;
            }
        }
    });
}

/// The columns, in order; the actions column has no sort.
const COLUMNS: [TicketSort; 5] = [
    TicketSort::Source,
    TicketSort::Project,
    TicketSort::Stage,
    TicketSort::Standing,
    TicketSort::Updated,
];

/// Every ticket the listing allows, one row each: the title is the
/// link, a header click sorts, and the last column acts.
fn ticket_table(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let core = cx.core;
    let p = theme::palette(ui);
    let mut listing = cx.state.dispatch_listing.clone();
    listing.project.clone_from(&cx.state.dispatch_project);
    let rows = core.tickets_listed(&listing);
    if rows.is_empty() {
        ui.label(theme::meta_text(ui, "No tickets match."));
        return;
    }
    // Rows hold buttons; a cell clips to the row, and a click outside
    // the clipped part of a button is no click.
    let height = ui.spacing().interact_size.y + 2.0 * ui.spacing().button_padding.y + 8.0;
    TableBuilder::new(ui)
        .id_salt("dispatch-tickets")
        .striped(true)
        .vscroll(false)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::remainder().at_least(200.0).clip(true))
        .column(Column::auto())
        .column(Column::auto())
        .column(Column::initial(240.0).at_least(80.0).clip(true))
        .column(Column::auto())
        .column(Column::auto())
        .header(height, |mut header| {
            for sort in COLUMNS {
                header.col(|ui| {
                    let arrow = if listing.sort == sort {
                        if listing.ascending { " ▲" } else { " ▼" }
                    } else {
                        ""
                    };
                    if theme::ghost_muted(ui, &format!("{}{arrow}", sort.label()))
                        .on_hover_text("Sort by this column; again to flip")
                        .clicked()
                    {
                        cx.state.dispatch_listing.sort_by(sort);
                    }
                });
            }
            header.col(|_| {});
        })
        .body(|mut body| {
            for t in rows {
                body.row(height, |mut row| {
                    row.col(|ui| {
                        if theme::ghost(ui, &title_of(t)).clicked() {
                            open_ticket(cx, &t.id);
                        }
                    });
                    row.col(|ui| {
                        ui.label(theme::meta_text(ui, &t.project));
                    });
                    row.col(|ui| {
                        ui.label(theme::meta_text(ui, crate::core::AppCore::ticket_stage(t)));
                    });
                    row.col(|ui| {
                        let standing = core.ticket_standing(t);
                        let urgent = core.ticket_waits(t) || t.state != "active";
                        ui.label(
                            RichText::new(&standing)
                                .text_style(theme::meta())
                                .color(if urgent { p.accent_2_text } else { p.n700 }),
                        )
                        .on_hover_text(standing);
                    });
                    row.col(|ui| {
                        ui.label(theme::meta_text(ui, since(t.updated_ms)));
                    });
                    row.col(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if core.ticket_waits(t) && theme::ghost(ui, "Answer").clicked() {
                            open_ticket(cx, &t.id);
                        }
                        if matches!(t.state.as_str(), "parked")
                            && theme::secondary(ui, "Resume").clicked()
                        {
                            cx.dispatch(AppAction::DispatchResume(t.id.clone()));
                        }
                        if theme::ghost_muted(ui, "Open").clicked() {
                            open_ticket(cx, &t.id);
                        }
                    });
                });
            }
        });
}

/// "3m", "2h", "5d" since a millisecond timestamp; nothing for zero.
fn since(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    super::cards::since_text(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
}

/// Pop out, or raise the window the page already has.
fn pop_out_button(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    if cx.state.surface == super::Surface::DispatchWindow {
        return;
    }
    ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
    if theme::ghost_muted(ui, "Pop out")
        .on_hover_text("Show Dispatch in a window of its own")
        .clicked()
    {
        let ctx = ui.ctx().clone();
        super::popout::raise_or_pop_out_dispatch(cx, &ctx);
    }
}

/// The main window's page while Dispatch has a window of its own.
pub fn elsewhere(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    theme::kicker(ui, "Dispatch", p.n600);
    ui.label(theme::meta_text(ui, "Dispatch is open in its own window."));
    if theme::secondary(ui, "Show its window").clicked() {
        let ctx = ui.ctx().clone();
        super::popout::raise_or_pop_out_dispatch(cx, &ctx);
    }
}

/// Go to a ticket: the window's own page inside the Dispatch window,
/// the main view otherwise.
fn open_ticket(cx: &mut DrawCtx<'_>, id: &str) {
    if cx.state.surface == super::Surface::DispatchWindow {
        cx.state.dispatch_window_ticket = Some(id.to_owned());
    } else {
        cx.dispatch(AppAction::ShowTicket(id.to_owned()));
    }
}

fn go_back(cx: &mut DrawCtx<'_>) {
    if cx.state.surface == super::Surface::DispatchWindow {
        cx.state.dispatch_window_ticket = None;
    } else {
        cx.dispatch(AppAction::Back);
    }
}

fn title_of(t: &TicketView) -> String {
    match t.number {
        Some(n) if t.kind == "pull-request" => format!("PR #{n} {}", t.title),
        Some(n) => format!("#{n} {}", t.title),
        None => t.title.clone(),
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
                    ui.label(theme::strong_text(&t.project));
                    ui.label(theme::meta_text(ui, "·"));
                    if theme::ghost(ui, &title_of(t)).clicked() {
                        open_ticket(cx, &t.id);
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
            if d.multiple {
                multiple_choice(cx, ui, d);
                return;
            }
            let note = {
                let draft = cx
                    .state
                    .dispatch_note_drafts
                    .entry(d.id.clone())
                    .or_default();
                ui.add(
                    egui::TextEdit::singleline(draft)
                        .hint_text("Note (optional)")
                        .desired_width(f32::INFINITY),
                )
                .on_hover_text("Sent with the answer; a rerun sends it to the agent");
                let text = draft.trim().to_owned();
                (!text.is_empty()).then_some(text)
            };
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
                        cx.state.dispatch_note_drafts.remove(&d.id);
                        cx.dispatch(AppAction::DispatchDecide {
                            ticket: d.ticket.clone(),
                            decision: d.id.clone(),
                            answer: option.clone(),
                            note: note.clone(),
                        });
                    }
                }
            });
        });
}

/// A decision that takes several options: a checkbox each, ticked from
/// the recommendation to start, and one Answer button that sends the
/// ticked ones joined by commas. Nothing is sent by a tick alone.
/// An agent at a prompt of its own: the ticket, the attempt, why, and
/// the session to open and answer it in.
fn agent_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, a: &WaitingAgent, ticket: Option<&TicketView>) {
    let p = theme::palette(ui);
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if let Some(t) = ticket {
                    if theme::ghost(ui, &title_of(t)).clicked() {
                        open_ticket(cx, &t.id);
                    }
                    ui.label(theme::meta_text(ui, "·"));
                    ui.label(theme::meta_text(ui, &t.project));
                    ui.label(theme::meta_text(ui, "·"));
                }
                ui.label(theme::strong_text(format!(
                    "{} ({}) agent",
                    a.stage, a.context
                )));
            });
            ui.label(RichText::new(&a.reason).color(p.accent_2_text));
            ui.horizontal(|ui| {
                if cx.core.at_trust_prompt(a.session)
                    && theme::secondary(ui, "Trust this folder")
                        .on_hover_text("Answer yes in the pane")
                        .clicked()
                {
                    cx.dispatch(AppAction::TrustFolder(a.session));
                }
                if theme::ghost(ui, "Open session")
                    .on_hover_text("The pane, to answer it there")
                    .clicked()
                {
                    cx.dispatch(AppAction::ShowSession(a.session));
                }
            });
        });
}

fn multiple_choice(cx: &mut DrawCtx<'_>, ui: &mut Ui, d: &DecisionView) {
    let key = format!("{}/{}", d.ticket, d.id);
    let suggested: Vec<String> = d
        .recommendation
        .as_deref()
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    let chosen = cx
        .state
        .dispatch_choices
        .entry(key)
        .or_insert_with(|| suggested.clone());
    ui.horizontal_wrapped(|ui| {
        for option in &d.options {
            let mut on = chosen.contains(option);
            let hint = if suggested.contains(option) {
                "Dispatch suggests this one"
            } else {
                "Tick every one that applies, then Answer"
            };
            if ui.checkbox(&mut on, option).on_hover_text(hint).changed() {
                if on {
                    chosen.push(option.clone());
                } else {
                    chosen.retain(|c| c != option);
                }
            }
        }
    });
    let ordered: Vec<String> = d
        .options
        .iter()
        .filter(|o| chosen.contains(o))
        .cloned()
        .collect();
    let answer = ordered.join(",");
    ui.horizontal(|ui| {
        let label = if ordered.is_empty() {
            "Answer".to_owned()
        } else {
            format!("Answer: {}", ordered.join(", "))
        };
        if ui
            .add_enabled_ui(!ordered.is_empty(), |ui| theme::primary(ui, &label))
            .inner
            .on_hover_text("Send the ticked options as the answer")
            .clicked()
        {
            cx.dispatch(AppAction::DispatchDecide {
                ticket: d.ticket.clone(),
                decision: d.id.clone(),
                answer,
                note: None,
            });
        }
        if ordered.is_empty() {
            ui.label(theme::meta_text(ui, "Tick at least one."));
        }
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
                .hint_text("decisions · status · take <project> 104 · !any shell line")
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
            go_back(cx);
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
                go_back(cx);
            }
            if t.state == "parked"
                && theme::secondary(ui, "Resume")
                    .on_hover_text("Back to active; the runner takes it from its current stage")
                    .clicked()
            {
                cx.dispatch(AppAction::DispatchResume(t.id.clone()));
            }
            if let Some(url) = &t.url {
                let what = if t.kind == "pull-request" {
                    "Pull request"
                } else {
                    "Issue"
                };
                ui.hyperlink_to(RichText::new(what).color(p.accent_text), url);
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
        ui.label(theme::strong_text(&t.project));
        ui.label(theme::meta_text(ui, "·"));
        stage_strip(ui, t);
        ui.label(theme::meta_text(ui, "·"));
        let agents = cx.core.waiting_agents_of(t);
        ui.label(
            RichText::new(cx.core.ticket_standing(t))
                .text_style(theme::meta())
                .color(if t.state == "active" && agents.is_empty() {
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

/// The pull request a `ready` attempt is bound to: a link when one was
/// found, and what its checks said at the head it was read at.
fn pr_labels(ui: &mut Ui, pr: &crate::ports::dispatch::PullRequestView) {
    let p = theme::palette(ui);
    if pr.number > 0 {
        ui.hyperlink_to(
            RichText::new(format!("PR #{}", pr.number))
                .text_style(theme::meta())
                .color(p.accent_text),
            &pr.url,
        );
    }
    let short: String = pr.head.chars().take(8).collect();
    ui.label(theme::meta_text(
        ui,
        if short.is_empty() {
            pr.checks.clone()
        } else {
            format!("{} at {short}", pr.checks)
        },
    ));
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
                if let Some(w) = cx
                    .core
                    .waiting_agents_of(t)
                    .into_iter()
                    .find(|w| w.stage == a.stage && w.context == a.context)
                {
                    ui.label(
                        RichText::new(format!("waiting on you: {}", w.reason))
                            .text_style(theme::meta())
                            .color(p.accent_2_text),
                    );
                }
                if let Some(checks) = &a.checks {
                    let short: String = checks.head.chars().take(8).collect();
                    ui.label(theme::meta_text(
                        ui,
                        match checks.exit {
                            None => format!("checks running at {short}"),
                            Some(0) => format!("checks passed at {short}"),
                            Some(code) => format!("checks exited {code} at {short}"),
                        },
                    ));
                }
                if let Some(pr) = &a.pr {
                    pr_labels(ui, pr);
                }
            });
            for round in &a.rounds {
                round_line(ui, round);
            }
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

/// One line per code review round: its head, its state and what each
/// reviewer said.
fn round_line(ui: &mut Ui, round: &crate::ports::dispatch::ReviewRoundView) {
    let p = theme::palette(ui);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let short: String = round.head.chars().take(8).collect();
        ui.label(theme::meta_text(
            ui,
            format!("round {} at {short}", round.n),
        ));
        let points = match round.open_points {
            0 => String::new(),
            n => format!(", {n} open"),
        };
        ui.label(
            RichText::new(format!("{}{points}", round.state))
                .text_style(theme::meta())
                .color(if round.state.starts_with("failed") {
                    p.accent_2_text
                } else {
                    p.n700
                }),
        );
        for (name, state) in &round.reviewers {
            ui.label(theme::meta_text(ui, format!("{name}: {state}")));
        }
        if let Some(after) = &round.head_after {
            let short: String = after.chars().take(8).collect();
            ui.label(theme::meta_text(ui, format!("fixed to {short}")));
        }
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
