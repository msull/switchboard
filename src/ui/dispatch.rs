//! The Dispatch page: every ticket with what waits on the user and a
//! console for `dispatch` commands; one ticket in full is `ticket.rs`.
//! Everything shown came through Dispatch's port as a view; a button is
//! an action the core turns into one call.

use std::path::PathBuf;

use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};

use super::{DrawCtx, GAP, theme};
use crate::core::dispatch::{parked, ticket_source, ticket_stage};
use crate::core::{
    AppAction, RunnerStanding, SupervisorState, TicketOnly, TicketSort, View, WaitingAgent,
};
use crate::ports::dispatch::{DecisionView, ProjectView, TicketView};

/// The console pane's height on the overview.
const CONSOLE_HEIGHT: f32 = 280.0;
/// The runner pane's height on the overview.
const RUNNER_HEIGHT: f32 = 220.0;

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);
    let state = cx.core.dispatch_state();
    let seen = state.seen;
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
        runner_row(cx, ui, tickets.len());
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
                .pending_decisions_in(chosen.as_deref())
                .into_iter()
                .cloned()
                .collect();
            let agents = cx.core.waiting_agents_in(chosen.as_deref());
            waiting_section(cx, ui, seen, &pending, &agents, &tickets);

            theme::section(ui, "Tickets");
            project_limits(cx, ui, projects.iter().filter(|p| shown(&p.name)));
            listing_controls(cx, ui);
            ticket_table(cx, ui);

            if let Some(record) = cx.core.runner().and_then(|id| cx.core.session(id).cloned()) {
                theme::section(ui, "Runner");
                runner(cx, ui, &record);
            }

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
/// allows and what holds new starts back, and its supervisor's chip.
fn project_limits<'a>(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    projects: impl Iterator<Item = &'a ProjectView>,
) {
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
            supervisor_chip(cx, ui, project);
        });
    }
    confirm_supervisor_fresh(cx, ui.ctx());
}

/// The project's supervisor: what it is doing, and Open, Resume, Trust
/// and Fresh where they apply. Nothing for a project without one.
fn supervisor_chip(cx: &mut DrawCtx<'_>, ui: &mut Ui, project: &ProjectView) {
    let Some(chip) = cx.core.supervisor_chip(project) else {
        return;
    };
    let Some(view) = project.supervisor.as_ref() else {
        return;
    };
    let p = theme::palette(ui);
    ui.label(theme::meta_text(ui, "·"));
    let mut words = format!("Supervisor · {}", chip.state.label());
    if view.seed_stale {
        words.push_str(" · seed changed");
    }
    if view.fresh_pending {
        words.push_str(" · starting");
    }
    ui.label(theme::meta_text(ui, words));
    if let Some(e) = &view.error {
        ui.label(
            RichText::new(e)
                .text_style(theme::meta())
                .color(p.accent_2_text),
        );
    }
    if let Some(id) = chip.session {
        if theme::ghost(ui, "Open supervisor")
            .on_hover_text("Show the supervisor's session")
            .clicked()
        {
            cx.dispatch(AppAction::ShowSession(id));
        }
        if chip.resumable
            && theme::ghost(ui, "Resume supervisor")
                .on_hover_text("Resume the supervisor's conversation (a paid run)")
                .clicked()
        {
            cx.dispatch(AppAction::ReturnToSession(id));
        }
        if chip.state == SupervisorState::AsksTrust
            && theme::ghost(ui, "Trust its folder")
                .on_hover_text("Answer Claude's trust question for the workspace with yes")
                .clicked()
        {
            cx.dispatch(AppAction::TrustFolder(id));
        }
    }
    if !view.fresh_pending
        && theme::ghost(ui, "Fresh supervisor")
            .on_hover_text("Start a new supervisor from the seed, replacing this one")
            .clicked()
    {
        cx.state.confirm_supervisor_fresh = Some(project.name.clone());
    }
}

/// The confirmation before a new supervisor replaces the current one,
/// since the current one is killed.
fn confirm_supervisor_fresh(cx: &mut DrawCtx<'_>, ctx: &egui::Context) {
    let Some(project) = cx.state.confirm_supervisor_fresh.clone() else {
        return;
    };
    let mut done = false;
    super::dialogs::dialog(ctx, "Start a new supervisor", |ui| {
        ui.label(format!(
            "{project}'s supervisor session is killed and kept in its history, its hand-off \
             is rotated, and a new one starts from the seed. Starting it is a paid run."
        ));
        let (confirmed, cancelled) = super::dialogs::dialog_actions(ui, "Start new", true);
        if confirmed {
            cx.dispatch(AppAction::DispatchSupervisorFresh(project.clone()));
            done = true;
        }
        if cancelled {
            done = true;
        }
    });
    if done || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cx.state.confirm_supervisor_fresh = None;
    }
}

/// The decisions and agents that wait on the user, as cards. A few are
/// shown in full; a pile of them folds behind its count, so the table
/// is not pushed off the page. The fold is chosen once, when the first
/// status arrives, and the user's clicks on it are remembered after.
fn waiting_section(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    seen: bool,
    pending: &[DecisionView],
    agents: &[WaitingAgent],
    tickets: &[TicketView],
) {
    let waiting = pending.len() + agents.len();
    let heading = if waiting == 0 {
        "Waiting on you".to_owned()
    } else {
        format!("Waiting on you · {waiting}")
    };
    let fold = ui.make_persistent_id("dispatch-waiting");
    let mut st =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), fold, true);
    if seen && !cx.state.dispatch_waiting_fold_set {
        st.set_open(waiting <= 3);
        cx.state.dispatch_waiting_fold_set = true;
    }
    // The heading toggles the fold too, not only the small arrow.
    let mut clicked = false;
    let mut header = st.show_header(ui, |ui| {
        clicked = ui
            .add(
                egui::Label::new(RichText::new(heading).text_style(theme::kicker_style()))
                    .sense(egui::Sense::click()),
            )
            .clicked();
    });
    if clicked {
        header.toggle();
    }
    header.body(|ui| {
        if waiting == 0 {
            ui.label(theme::meta_text(ui, "Nothing waits on you."));
        }
        for d in pending {
            let ticket = tickets.iter().find(|t| t.id == d.ticket).cloned();
            decision_card(cx, ui, d, ticket.as_ref(), true);
        }
        for a in agents {
            let ticket = tickets.iter().find(|t| t.id == a.ticket).cloned();
            agent_card(cx, ui, a, ticket.as_ref());
        }
    });
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
    let now = super::cards::now_ms();
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
                        ui.label(theme::meta_text(ui, ticket_stage(t)));
                    });
                    row.col(|ui| {
                        let standing = core.ticket_standing(t);
                        ui.label(RichText::new(&standing).text_style(theme::meta()).color(
                            if core.ticket_urgent(t) {
                                p.accent_2_text
                            } else {
                                p.n700
                            },
                        ))
                        .on_hover_text(standing);
                    });
                    row.col(|ui| {
                        let label = ui.label(theme::meta_text(
                            ui,
                            super::cards::ago_ms(t.updated_ms, now),
                        ));
                        if t.updated_ms > 0 {
                            label.on_hover_text(super::cards::at_local(t.updated_ms));
                        }
                    });
                    row.col(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if core.ticket_waits(t) && theme::ghost(ui, "Answer").clicked() {
                            open_ticket(cx, &t.id);
                        }
                        if parked(t)
                            && theme::secondary(ui, "Resume")
                                .on_hover_text(RESUME_HINT)
                                .clicked()
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
pub(super) fn open_ticket(cx: &mut DrawCtx<'_>, id: &str) {
    if cx.state.surface == super::Surface::DispatchWindow {
        cx.state.dispatch_window_ticket = Some(id.to_owned());
    } else {
        cx.dispatch(AppAction::ShowTicket(id.to_owned()));
    }
}

/// The ticket's number and title, as links and headings name it.
pub(super) fn title_of(t: &TicketView) -> String {
    match ticket_source(t) {
        source if source.is_empty() => t.title.clone(),
        source => format!("{source} {}", t.title),
    }
}

/// A decision with its options as buttons, the recommended one filled.
/// `with_ticket` names the ticket above the question, for the overview.
pub(super) fn decision_card(
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
                let by = d
                    .answered_by
                    .as_ref()
                    .map_or(String::new(), |by| format!(" by {by}"));
                ui.label(
                    RichText::new(format!(
                        "{}: {}{by}",
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

/// A decision that takes several options: a checkbox each, ticked from
/// the recommendation to start, and one Answer button that sends the
/// ticked ones joined by commas. Nothing is sent by a tick alone.
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

/// Where the runner stands, and the toggle: Stop while the runner is
/// stoppable (its pane runs, or a Start is queued behind a Stop), Start
/// otherwise. Start is what relaunches it on every app
/// start, so its hover says so.
fn runner_row(cx: &mut DrawCtx<'_>, ui: &mut Ui, tickets: usize) {
    let p = theme::palette(ui);
    let standing = cx.core.runner_standing();
    let text = match standing {
        RunnerStanding::Up { pid } => {
            let pid = pid.map(|pid| format!(" · pid {pid}")).unwrap_or_default();
            format!("runner up{pid} · {tickets} ticket(s)")
        }
        RunnerStanding::Starting => "runner starting…".to_owned(),
        RunnerStanding::StartQueued => "starting after the old runner stops…".to_owned(),
        RunnerStanding::Stopping => "runner stopping…".to_owned(),
        RunnerStanding::Outside => format!("runner up outside the app · {tickets} ticket(s)"),
        RunnerStanding::Gone => "runner gone; showing its last status".to_owned(),
        RunnerStanding::Stopped => "runner stopped".to_owned(),
    };
    ui.label(
        RichText::new(text)
            .text_style(theme::meta())
            .color(if standing.up() {
                p.n700
            } else {
                p.accent_2_text
            }),
    );
    if standing.stoppable() {
        if theme::ghost_muted(ui, "Stop")
            .on_hover_text(
                "Kill the runner; it stays stopped across app starts until Start. \
                 After a rebundle, Stop then Start to run the new build",
            )
            .clicked()
        {
            cx.dispatch(AppAction::DispatchRunnerStop);
        }
    } else {
        let refusal = cx.core.runner_refusal();
        let hint = refusal.map_or_else(
            || {
                format!(
                    "Run `dispatch run` in {}. Started, it comes back when the app starts, \
                     and active tickets' agents start with it",
                    cx.core.dispatch_state().data_dir.display()
                )
            },
            str::to_owned,
        );
        if ui
            .add_enabled_ui(refusal.is_none(), |ui| theme::ghost(ui, "▶ Start"))
            .inner
            .on_hover_text(hint)
            .clicked()
        {
            cx.dispatch(AppAction::DispatchRunnerStart);
        }
        let record = cx.core.runner().and_then(|id| cx.core.session(id));
        if let Some(record) = record.filter(|r| r.last_run().is_some()) {
            let last = super::runs::kicker(record, false, std::time::SystemTime::now());
            ui.label(theme::meta_text(ui, format!("last run: {last}")));
        }
    }
}

/// The runner's pane while it runs; stopped, the tail of its last run's
/// log, so an exit says why.
fn runner(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &crate::core::SessionRecord) {
    if cx.core.is_running(record.id) {
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), RUNNER_HEIGHT),
            egui::Layout::top_down(egui::Align::Min),
            |ui| super::session::live_pane(cx, ui, record),
        );
    } else if record.last_run().is_some() {
        super::runs::last_output(cx, ui, record, RUNNER_HEIGHT);
    }
}

/// What a Resume click spends, on both Resume buttons: it is
/// `dispatch resume`, which reruns what the park cancelled mid-run.
pub(super) const RESUME_HINT: &str = "Back to active; what the park cancelled mid-run runs again (a paid run), and the rest is asked";

/// Whether `view` is one of Dispatch's, for the rail.
#[must_use]
pub fn is_dispatch_view(view: &View) -> bool {
    matches!(view, View::Dispatch | View::Ticket(_))
}
