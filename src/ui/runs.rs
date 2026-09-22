//! Commands and services as runs: the one action row every card and
//! list uses (the name opens, `▶ Run` runs, `Stop` stops), the kicker
//! that says when the last run happened and how it ended, the working
//! set card with the output kept on it and its artifacts, and the
//! record's page with the run history.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use egui::{RichText, Sense, Ui, UiBuilder, vec2};

use super::cards::{file_name, is_running, removable, since_text};
use super::{DrawCtx, GAP, document, theme};
use crate::core::{AppAction, Run, SessionKind, SessionRecord};

/// Lines of a run's log the page reads.
const LOG_LINES: usize = 2000;
/// Lines of the last run's output a board card shows.
const CARD_LINES: usize = 3;

/// `1.2 s`, `45 s`, `3 min 2 s`, `1 h 4 min`.
#[must_use]
pub fn duration_text(d: Duration) -> String {
    let secs = d.as_secs();
    match secs {
        s if s < 10 => format!("{:.1} s", d.as_secs_f64()),
        s if s < 60 => format!("{s} s"),
        s if s < 3600 => format!("{} min {} s", s / 60, s % 60),
        s => format!("{} h {} min", s / 3600, (s % 3600) / 60),
    }
}

/// The run line of a card's kicker: `ok · 1.2 s · 3m ago`,
/// `exit 2 · 5 s · just now`, `running · 12 s`, or `never run`.
#[must_use]
pub fn kicker(record: &SessionRecord, running: bool, now: SystemTime) -> String {
    let Some(run) = record.last_run() else {
        return if running {
            "running".into()
        } else {
            "never run".into()
        };
    };
    let took = run.duration(now).map(duration_text).unwrap_or_default();
    if running && run.open() {
        return format!("running · {took}");
    }
    let ended = run.ended.unwrap_or(run.started);
    let how = match run.exit {
        Some(0) => "ok".to_owned(),
        Some(code) => format!("exit {code}"),
        None if run.open() => "stopped".to_owned(),
        None => "killed".to_owned(),
    };
    let when = match since_text(ended) {
        s if s == "just now" => s,
        s => format!("{s} ago"),
    };
    format!("{how} · {took} · {when}")
}

/// The one action row for commands and services: run or stop, open,
/// remove. Nothing else on a card starts a run.
pub fn actions(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, running: bool) {
    ui.spacing_mut().item_spacing.x = 14.0;
    ui.spacing_mut().button_padding = vec2(0.0, 4.0);
    let id = record.id;
    let service = record.kind == SessionKind::Service;
    if running {
        if theme::ghost_muted(ui, "Stop")
            .on_hover_text("Kill the process; its output so far is kept")
            .clicked()
        {
            cx.dispatch(AppAction::KillSession(id));
        }
    } else {
        let label = if service { "▶ Start" } else { "▶ Run" };
        let hint = if record.runnable() {
            "Run it now; every run keeps its own output"
        } else {
            "Not approved yet: read its definition on the Run tab first"
        };
        if ui
            .add_enabled_ui(record.runnable(), |ui| theme::ghost(ui, label))
            .inner
            .on_hover_text(hint)
            .clicked()
        {
            cx.dispatch(AppAction::RestartSession(id));
        }
    }
    if theme::ghost(ui, "Open")
        .on_hover_text("The run history and the full output")
        .clicked()
    {
        cx.dispatch(AppAction::ShowSession(id));
    }
    if !running && removable(record) && theme::ghost_muted(ui, "Remove").clicked() {
        cx.dispatch(AppAction::RemoveSession(id));
    }
}

/// What a working-set command card shows in its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunCardMode {
    /// The last run's output.
    #[default]
    Output,
    /// One of the last run's artifacts, by index.
    Artifact(usize),
}

/// Where a run's log lives.
fn log_path(data_dir: &Path, run: &Run) -> PathBuf {
    data_dir.join("scrollback").join(&run.log)
}

/// A run's output, read from its log and cached until the file changes.
/// A run in progress reads fresh every time it is drawn (the file grows).
fn run_log<'a>(
    logs: &'a mut HashMap<(crate::core::RecordId, u32), (Option<SystemTime>, String)>,
    data_dir: &Path,
    record: &SessionRecord,
    run: &Run,
) -> &'a str {
    let path = log_path(data_dir, run);
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let key = (record.id, run.n);
    let stale = logs.get(&key).is_none_or(|(m, _)| *m != modified);
    if stale {
        let text = crate::adapters::scrollback::tail_text(&path, LOG_LINES).unwrap_or_default();
        logs.insert(key, (modified, text));
    }
    logs.get(&key).map_or("", |(_, t)| t.as_str())
}

/// The artifacts of the last run as chips; a click on one returns its
/// index.
fn artifact_chips(ui: &mut Ui, run: &Run, selected: Option<usize>) -> Option<usize> {
    let p = theme::palette(ui);
    let mut clicked = None;
    if run.artifacts.is_empty() {
        return None;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spacing_mut().button_padding = vec2(6.0, 2.0);
        for (i, path) in run.artifacts.iter().enumerate() {
            let name = file_name(path);
            let text = RichText::new(name).text_style(theme::meta());
            let text = if selected == Some(i) {
                text.color(p.accent_text)
            } else {
                text.color(p.n700)
            };
            if ui
                .add(egui::Button::new(text).frame_when_inactive(false))
                .on_hover_text(path.display().to_string())
                .clicked()
            {
                clicked = Some(i);
            }
        }
    });
    clicked
}

/// A board card's body for an entry: the command line, the last run's
/// last lines, its artifacts (a click previews the file).
pub fn card_body(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    if let crate::core::Launch::Command { command, .. } = &record.launch {
        ui.add(egui::Label::new(theme::mono_text(ui, command)).truncate());
    }
    let Some(run) = record.last_run().cloned() else {
        ui.label(theme::meta_text(ui, "No runs yet."));
        return;
    };
    let data_dir = cx.services.store.data_dir();
    let tail = super::working_set::pane_tail(
        run_log(&mut cx.state.run_logs, &data_dir, record, &run),
        CARD_LINES,
    );
    if !tail.is_empty() {
        ui.add(egui::Label::new(RichText::new(tail).monospace().small().color(p.n800)).truncate());
    }
    if let Some(i) = artifact_chips(ui, &run, None)
        && let Some(path) = run.artifacts.get(i)
    {
        cx.dispatch(AppAction::ShowDocument(record.project, path.clone()));
    }
}

/// The working-set card of a command or service: kicker, title, the
/// output kept in the body (or one artifact, previewed), the chips, and
/// the action row.
pub fn set_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    let state = cx.core.card_state(record.id);
    let running = is_running(cx.core, record.id);
    let project = cx
        .core
        .workspace(record.project)
        .map(|w| w.project.name.clone())
        .unwrap_or_default();
    let mut open = false;
    let last = record.last_run().cloned();
    let mode = cx
        .state
        .run_modes
        .get(&record.id)
        .copied()
        .unwrap_or_default();
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.style_mut().interaction.selectable_labels = false;
            ui.horizontal(|ui| {
                theme::kicker(
                    ui,
                    &format!("{} · {project}", kicker(record, running, SystemTime::now())),
                    p.state_text(&state),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if running {
                        ui.add(egui::Spinner::new().size(10.0));
                    } else {
                        theme::status_dot(ui, &state, 8.0);
                    }
                });
            });
            let title = ui
                .add(
                    egui::Label::new(RichText::new(&record.name).text_style(theme::card_title()))
                        .truncate()
                        .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            open = title.clicked();
            title.context_menu(|ui| {
                if super::working_set::set_menu(cx, ui, &crate::core::PinTarget::Session(record.id))
                {
                    ui.close();
                }
            });
            if let crate::core::Launch::Command { command, .. } = &record.launch {
                ui.add(egui::Label::new(theme::mono_text(ui, command).small()).truncate());
            }
            let chips_height = if last.as_ref().is_some_and(|r| !r.artifacts.is_empty()) {
                30.0
            } else {
                0.0
            };
            let body_height = (ui.available_height() - 40.0 - chips_height).max(20.0);
            let body_rect =
                egui::Rect::from_min_size(ui.cursor().min, vec2(ui.available_width(), body_height));
            let mut body = ui.new_child(UiBuilder::new().max_rect(body_rect));
            body.set_clip_rect(body_rect.intersect(ui.clip_rect()));
            set_card_body(cx, &mut body, record, last.as_ref(), mode);
            ui.advance_cursor_after_rect(body_rect);
            if let Some(run) = &last {
                let selected = match mode {
                    RunCardMode::Artifact(i) => Some(i),
                    RunCardMode::Output => None,
                };
                if let Some(i) = artifact_chips(ui, run, selected) {
                    let next = if selected == Some(i) {
                        RunCardMode::Output
                    } else {
                        RunCardMode::Artifact(i)
                    };
                    cx.state.run_modes.insert(record.id, next);
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| actions(cx, ui, record, running));
            });
        });
    if open {
        cx.dispatch(AppAction::ShowSession(record.id));
    }
}

/// The body of a set card: the output (sticking to its end while the
/// run goes) or the chosen artifact.
fn set_card_body(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    record: &SessionRecord,
    last: Option<&Run>,
    mode: RunCardMode,
) {
    let Some(run) = last else {
        ui.label(theme::meta_text(ui, "No runs yet. ▶ Run starts one."));
        return;
    };
    let data_dir = cx.services.store.data_dir();
    if let RunCardMode::Artifact(i) = mode
        && let Some(path) = run.artifacts.get(i)
    {
        let renders = data_dir.join("renders");
        egui::ScrollArea::vertical()
            .id_salt(("artifact", record.id, i))
            .auto_shrink(false)
            .show(ui, |ui| document::show_file(cx.state, ui, path, &renders));
        return;
    }
    let text = run_log(&mut cx.state.run_logs, &data_dir, record, run).to_owned();
    if text.is_empty() {
        ui.label(theme::meta_text(ui, "No output yet."));
        return;
    }
    egui::ScrollArea::both()
        .id_salt(("run-output", record.id, run.n))
        .auto_shrink(false)
        .stick_to_bottom(run.open())
        .show(ui, |ui| super::session::code_block(ui, &text));
}

/// Width of the run list at the page's left.
const RUNS_WIDTH: f32 = 190.0;

/// The page of a command or service under its header: the runs at the
/// left, the selected run's log (or the live pane for the one running)
/// in the middle, and its artifacts at the right.
pub fn page(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let running = is_running(cx.core, record.id);
    if record.runs.is_empty() {
        if running {
            super::session::live_pane(cx, ui, record);
        } else if let Some(text) = cx.state.snapshots.get(&record.id).cloned() {
            // A record from before runs were kept: its one log, as the
            // shell reads it for the open session.
            ui.label(theme::meta_text(
                ui,
                "Not running. Below is the last output kept on disk.",
            ));
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| super::session::code_block(ui, &text));
        } else {
            ui.label(theme::meta_text(ui, "No runs yet. ▶ Run starts one."));
        }
        return;
    }
    let selected = cx
        .state
        .run_selected
        .get(&record.id)
        .and_then(|n| record.runs.iter().find(|r| r.n == *n))
        .or(record.last_run())
        .cloned();
    let Some(run) = selected else {
        return;
    };
    let is_latest = record.last_run().is_some_and(|r| r.n == run.n);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(
            vec2(RUNS_WIDTH, ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| run_list(cx, ui, record, run.n),
        );
        ui.add_space(GAP);
        let live = is_latest && running && run.open();
        if run.artifacts.is_empty() {
            ui.allocate_ui_with_layout(
                vec2(ui.available_width(), ui.available_height()),
                egui::Layout::top_down(egui::Align::Min),
                |ui| output_view(cx, ui, record, &run, live),
            );
            return;
        }
        // Output above the files, the full width for each, so a page of
        // a PDF or a wide table reads; the split is dragged to taste.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), ui.available_height()),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                let total = ui.available_height();
                egui::Panel::top(egui::Id::new(("run-output", record.id)))
                    .resizable(true)
                    .default_size((total * 0.35).max(120.0))
                    .size_range(80.0..=(total - 120.0).max(80.0))
                    .show_separator_line(false)
                    .frame(egui::Frame::new().inner_margin(egui::Margin::ZERO))
                    .show(ui, |ui| output_view(cx, ui, record, &run, live));
                ui.add_space(GAP);
                artifacts_view(cx, ui, record, &run);
            },
        );
    });
}

/// The run's output: the live pane while it is the run in progress,
/// else its kept log.
fn output_view(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, run: &Run, live: bool) {
    if live {
        super::session::live_pane(cx, ui, record);
    } else {
        log_view(cx, ui, record, run);
    }
}

fn run_list(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, selected: u32) {
    let p = theme::palette(ui);
    theme::section(ui, "Runs");
    let now = SystemTime::now();
    egui::ScrollArea::vertical()
        .id_salt(("runs", record.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            for run in record.runs.iter().rev() {
                let when = started_text(run.started);
                let took = run.duration(now).map(duration_text).unwrap_or_default();
                let how = match (run.exit, run.open()) {
                    (Some(0), _) => "ok".to_owned(),
                    (Some(code), _) => format!("exit {code}"),
                    (None, true) => "running".to_owned(),
                    (None, false) => "killed".to_owned(),
                };
                let label = format!("Run {}", run.n);
                let text = if run.n == selected {
                    RichText::new(&label).text_style(theme::strong())
                } else {
                    RichText::new(&label)
                };
                if ui.add(egui::Button::new(text).frame(false)).clicked() {
                    cx.state.run_selected.insert(record.id, run.n);
                }
                ui.label(
                    RichText::new(format!("{when} · {took} · {how}"))
                        .text_style(theme::meta())
                        .color(match run.exit {
                            Some(0) | None => p.n600,
                            Some(_) => p.accent_2_text,
                        }),
                );
                if !run.artifacts.is_empty() {
                    ui.label(
                        RichText::new(format!("{} file(s)", run.artifacts.len()))
                            .text_style(theme::meta())
                            .color(p.n600),
                    );
                }
                ui.add_space(4.0);
            }
        });
}

/// A start time as a clock reading today, or a date and time.
fn started_text(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let local = chrono_lite(secs);
    let today = chrono_lite(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    );
    if local.0 == today.0 {
        format!("{:02}:{:02}", local.1, local.2)
    } else {
        format!("{} {:02}:{:02}", local.0, local.1, local.2)
    }
}

/// `(YYYY-MM-DD, hour, minute)` in local time, without a date crate:
/// the zone offset comes from `date` once, the rest is arithmetic.
fn chrono_lite(secs: u64) -> (String, u64, u64) {
    civil(i64::try_from(secs).unwrap_or(0) + local_offset_secs())
}

/// The civil date and time of a count of seconds since the epoch that
/// already includes the zone offset.
fn civil(local: i64) -> (String, u64, u64) {
    let days = local.div_euclid(86_400);
    let rem = local.rem_euclid(86_400);
    let (hour, minute) = (rem / 3600, (rem % 3600) / 60);
    // Civil-from-days (Howard Hinnant), valid for the range we show.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let doe = shifted.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (
        format!("{year:04}-{month:02}-{day:02}"),
        u64::try_from(hour).unwrap_or(0),
        u64::try_from(minute).unwrap_or(0),
    )
}

/// The local zone's offset from UTC in seconds, read once from `date`.
fn local_offset_secs() -> i64 {
    use std::sync::OnceLock;
    static OFFSET: OnceLock<i64> = OnceLock::new();
    *OFFSET.get_or_init(|| {
        let out = std::process::Command::new("date").arg("+%z").output().ok();
        let text = out
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        parse_offset(&text).unwrap_or(0)
    })
}

/// `+0530` -> 19800, `-0700` -> -25200.
fn parse_offset(text: &str) -> Option<i64> {
    let (sign, digits) = text.split_at(1);
    let sign = match sign {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    if digits.len() != 4 {
        return None;
    }
    let h: i64 = digits[..2].parse().ok()?;
    let m: i64 = digits[2..].parse().ok()?;
    Some(sign * (h * 3600 + m * 60))
}

fn log_view(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, run: &Run) {
    theme::section(ui, "Output");
    let data_dir = cx.services.store.data_dir();
    let text = run_log(&mut cx.state.run_logs, &data_dir, record, run).to_owned();
    if text.is_empty() {
        ui.label(theme::meta_text(ui, "No output was kept for this run."));
        return;
    }
    egui::ScrollArea::both()
        .id_salt(("run-log", record.id, run.n))
        .auto_shrink(false)
        .show(ui, |ui| super::session::code_block(ui, &text));
}

fn artifacts_view(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, run: &Run) {
    theme::section(ui, "Files");
    let selected = match cx.state.run_modes.get(&record.id) {
        Some(RunCardMode::Artifact(i)) => Some(*i),
        _ => None,
    }
    .filter(|i| *i < run.artifacts.len())
    .unwrap_or(0);
    if let Some(i) = artifact_chips(ui, run, Some(selected)) {
        cx.state
            .run_modes
            .insert(record.id, RunCardMode::Artifact(i));
    }
    let Some(path) = run.artifacts.get(selected) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().button_padding = vec2(6.0, 3.0);
        if theme::ghost(ui, "Open in app").clicked() {
            cx.dispatch(AppAction::OpenDocument(path.clone()));
        }
        if theme::ghost_muted(ui, "Reveal").clicked() {
            cx.dispatch(AppAction::RevealDocument(path.clone()));
        }
    });
    let renders = cx.services.store.data_dir().join("renders");
    egui::ScrollArea::vertical()
        .id_salt(("artifact-page", record.id, selected))
        .auto_shrink(false)
        .show(ui, |ui| document::show_file(cx.state, ui, path, &renders));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration_text(Duration::from_millis(1234)), "1.2 s");
        assert_eq!(duration_text(Duration::from_secs(45)), "45 s");
        assert_eq!(duration_text(Duration::from_secs(182)), "3 min 2 s");
        assert_eq!(duration_text(Duration::from_mins(64)), "1 h 4 min");
    }

    #[test]
    fn offsets_parse() {
        assert_eq!(parse_offset("+0530"), Some(19_800));
        assert_eq!(parse_offset("-0700"), Some(-25_200));
        assert_eq!(parse_offset("x"), None);
    }

    #[test]
    fn civil_dates_come_out_right() {
        assert_eq!(civil(1_789_993_000), ("2026-09-21".into(), 12, 16));
        assert_eq!(civil(0), ("1970-01-01".into(), 0, 0));
        assert_eq!(civil(951_782_400), ("2000-02-29".into(), 0, 0));
    }
}
