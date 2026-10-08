//! The card: one widget for every board entry (agent, shell, command,
//! service), differing only by its kicker, glyph, and actions. Also the
//! grid that lays cards out and the text helpers the views share.

use std::path::Path;
use std::time::SystemTime;

use egui::{RichText, Sense, Ui, vec2};

use super::{DrawCtx, theme};
use crate::core::{
    AppAction, Approval, CardState, PinTarget, ProjectId, RecordId, SessionKind, SessionRecord,
};
use crate::ports::transcript::Conversation;

/// Cards are at least this wide; the grid adds columns as room allows.
const MIN_CARD_WIDTH: f32 = 230.0;
/// Agent and shell cards; command and service cards are shorter.
pub const SESSION_CARD_HEIGHT: f32 = 172.0;
pub const ENTRY_CARD_HEIGHT: f32 = 184.0;
const GRID_GAP: f32 = 14.0;

#[must_use]
pub fn kind_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Agent(agent) => agent.label(),
        SessionKind::Command => "command",
        SessionKind::Service => "service",
        SessionKind::Shell => "shell",
    }
}

/// Last path component, or the whole path when there is none.
#[must_use]
pub fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// "3m", "2h", "5d": how long ago `then` was. The UI may read the wall
/// clock; the core may not.
#[must_use]
pub fn since_text(then: SystemTime) -> String {
    age(then, SystemTime::now()).unwrap_or_else(|| "just now".into())
}

/// `since_text` as a phrase measured at `now`: "3m ago", or "just now".
#[must_use]
pub fn ago_text(then: SystemTime, now: SystemTime) -> String {
    age(then, now).map_or_else(|| "just now".into(), |age| format!("{age} ago"))
}

/// The age of `then` at `now` in its largest whole unit, or `None`
/// under a minute (and for a time in the future).
fn age(then: SystemTime, now: SystemTime) -> Option<String> {
    let secs = now.duration_since(then).map_or(0, |d| d.as_secs());
    match secs {
        s if s < 60 => None,
        s if s < 3600 => Some(format!("{}m", s / 60)),
        s if s < 86_400 => Some(format!("{}h", s / 3600)),
        s => Some(format!("{}d", s / 86_400)),
    }
}

/// How long ago `then_ms` was at `now_ms`, in words: "just now" under
/// a minute (and for a time in the future), "N min ago" under an hour,
/// "N h M min ago" under a day ("N h ago" on the hour), then "N d ago".
/// Empty for zero, which is how a view says it has no time.
///
/// Dispatch's pages have their own form rather than `ago_text`'s: their
/// times are Dispatch's epoch milliseconds, and a timeline lists many
/// rows within the same hour, which "1h ago" would make look alike. The
/// ticket table uses it too, so a ticket reads the same in the list and
/// on its page.
#[must_use]
pub fn ago_ms(then_ms: u64, now_ms: u64) -> String {
    if then_ms == 0 {
        return String::new();
    }
    let mins = now_ms.saturating_sub(then_ms) / 60_000;
    match mins {
        0 => "just now".to_owned(),
        m if m < 60 => format!("{m} min ago"),
        m if m < 24 * 60 => match m % 60 {
            0 => format!("{} h ago", m / 60),
            rest => format!("{} h {rest} min ago", m / 60),
        },
        m => format!("{} d ago", m / (24 * 60)),
    }
}

/// A millisecond timestamp as local date and time, for the hover on a
/// relative time.
#[must_use]
pub fn at_local(ms: u64) -> String {
    let t = SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(ms);
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// Now, in milliseconds since the epoch: the page's clock for `ago_ms`.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Lay `count` cells out in a grid whose columns are as many
/// `MIN_CARD_WIDTH` cards as fit the width, 14 px apart, every cell
/// `height` tall. `cell` draws cell `i`.
pub fn grid(ui: &mut Ui, count: usize, height: f32, mut cell: impl FnMut(&mut Ui, usize)) {
    let width = ui.available_width();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let columns = (((width + GRID_GAP) / (MIN_CARD_WIDTH + GRID_GAP)).floor() as usize).max(1);
    #[allow(clippy::cast_precision_loss)]
    let cell_width = (width - GRID_GAP * (columns as f32 - 1.0)) / columns as f32;
    let mut i = 0;
    while i < count {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GRID_GAP;
            for _ in 0..columns {
                if i >= count {
                    break;
                }
                ui.allocate_ui_with_layout(
                    vec2(cell_width, height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_size(vec2(cell_width, height));
                        ui.set_max_width(cell_width);
                        cell(ui, i);
                    },
                );
                i += 1;
            }
        });
        ui.add_space(GRID_GAP - ui.spacing().item_spacing.y);
    }
}

/// The kicker line of a card: the state, and for commands and services
/// the kind before it. The reason a session waits is the body instead.
pub(super) fn kicker_text(record: &SessionRecord, state: &CardState, running: bool) -> String {
    let age = if running {
        since_text(record.last_seen)
    } else {
        since_text(record.created)
    };
    let label = state.label();
    match record.kind {
        SessionKind::Command | SessionKind::Service => {
            format!("{} · {label}", kind_label(record.kind))
        }
        SessionKind::Agent(_) | SessionKind::Shell => match state {
            CardState::WaitingOnYou => label,
            _ => format!("{label} · {age}"),
        },
    }
}

/// One card. Its title opens the session; the buttons act on it without
/// opening. `not running` cards are outlined instead of filled.
pub fn session_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord) {
    let p = theme::palette(ui);
    let state = cx.core.card_state(record.id);
    let running = cx.core.is_running(record.id);
    let entry = matches!(record.kind, SessionKind::Command | SessionKind::Service);
    let hollow = !running && !entry;
    let conversation = cx.state.conversations.get(&record.id).map(|(_, c)| c);
    // An agent's pane ends in its own chrome (prompt hints, token
    // counts), so its excerpt comes from the transcript instead.
    let caption = if matches!(record.kind, SessionKind::Agent(_)) {
        conversation.and_then(agent_excerpt)
    } else {
        cx.state
            .captions
            .get(&record.id)
            .cloned()
            .or_else(|| cx.core.host_status(record.id).and_then(|h| h.title.clone()))
    };
    let model = conversation.and_then(|c| c.model.clone());
    let kicker = if entry {
        format!(
            "{} · {}",
            kind_label(record.kind),
            super::runs::kicker(record, running, SystemTime::now())
        )
    } else {
        kicker_text(record, &state, running)
    };
    let reason = (state == CardState::WaitingOnYou)
        .then(|| cx.core.waiting_reason(record.id))
        .flatten();

    let mut frame = egui::Frame::new()
        .corner_radius(2)
        .inner_margin(egui::Margin::symmetric(14, 12));
    frame = if hollow {
        frame.stroke(egui::Stroke::new(1.0, p.n300))
    } else {
        frame.fill(p.surface)
    };
    let mut open = false;
    frame.show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        // Text on a card is not selectable; otherwise a click on the
        // name would start a selection instead of opening the session.
        ui.style_mut().interaction.selectable_labels = false;
        ui.horizontal(|ui| {
            theme::kicker(ui, &kicker, p.state_text(&state));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if record.kind == SessionKind::Command {
                    ui.label(RichText::new("▶").small().color(p.n600));
                } else {
                    theme::status_dot(ui, &state, 8.0);
                }
            });
        });
        let title_color = if hollow { p.n700 } else { p.text };
        let title = ui
            .add(
                egui::Label::new(
                    RichText::new(&record.name)
                        .text_style(theme::card_title())
                        .color(title_color),
                )
                .truncate()
                .sense(Sense::click()),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        open = title.clicked();
        title.context_menu(|ui| {
            if super::working_set::set_menu(cx, ui, &PinTarget::Session(record.id)) {
                ui.close();
            }
        });
        if entry {
            super::runs::card_body(cx, ui, record);
        } else {
            let ask = cx.core.ask_beside_reason(record.id);
            card_body(ui, record, model, reason, ask, caption.as_deref());
        }
        if !running && !entry && state == CardState::NotResumable {
            ui.label(
                RichText::new("not resumable")
                    .small()
                    .color(p.accent_2_text),
            );
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            actions(cx, ui, record, running);
        });
    });
    if open {
        cx.dispatch(AppAction::ShowSession(record.id));
    }
}

/// The meta line and the body of an agent's or a shell's card: kind,
/// model, and directory, then why the session waits and what it asked,
/// or else the pane's last line.
fn card_body(
    ui: &mut Ui,
    record: &SessionRecord,
    model: Option<String>,
    reason: Option<String>,
    ask: Option<&str>,
    caption: Option<&str>,
) {
    let p = theme::palette(ui);
    let mut parts = vec![kind_label(record.kind).to_owned()];
    parts.extend(model);
    parts.push(file_name(&record.cwd));
    ui.add(egui::Label::new(RichText::new(parts.join(" · ")).small().color(p.n600)).truncate());
    // The body: why the session waits, or else the excerpt, clamped to
    // two lines so the action row below keeps its place.
    let body_style = theme::excerpt();
    if let Some(ask) = ask {
        // A permission prompt can block the pane while the question
        // stands; both are the owner's to answer.
        if let Some(reason) = reason {
            ui.add(
                egui::Label::new(RichText::new(reason).small().color(p.accent_2_text)).truncate(),
            );
        }
        let text = clamp_lines(ui, ask, &theme::meta(), 2);
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            )
            .wrap(),
        );
        return;
    }
    if let Some(reason) = reason {
        let text = clamp_lines(ui, &reason, &theme::meta(), 2);
        ui.add(
            egui::Label::new(
                RichText::new(text)
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            )
            .wrap(),
        );
        return;
    }
    let body = caption.map(last_line);
    if let Some(body) = body.filter(|b| !b.is_empty()) {
        let text = clamp_lines(ui, &body, &body_style, 2);
        ui.add(egui::Label::new(RichText::new(text).text_style(body_style).color(p.n800)).wrap());
    }
}

/// `text` cut so that, wrapped at the current width in `style`, it takes
/// at most `lines` rows, with an ellipsis where it was cut.
fn clamp_lines(ui: &Ui, text: &str, style: &egui::TextStyle, lines: usize) -> String {
    let font = style.resolve(ui.style());
    let width = ui.available_width();
    let galley = ui
        .painter()
        .layout(text.to_owned(), font, egui::Color32::BLACK, width);
    if galley.rows.len() <= lines {
        return text.to_owned();
    }
    // Keep the glyphs of the first rows, less room for the ellipsis.
    let keep: usize = galley.rows.iter().take(lines).map(|r| r.glyphs.len()).sum();
    let cut: String = text.chars().take(keep.saturating_sub(2)).collect();
    format!("{}…", cut.trim_end())
}

/// The action row: ghost buttons flush left, destructive ones last in
/// neutral.
pub(super) fn actions(cx: &mut DrawCtx<'_>, ui: &mut Ui, record: &SessionRecord, running: bool) {
    ui.horizontal(|ui| {
        super::action_spacing(ui);
        let id = record.id;
        match record.kind {
            SessionKind::Agent(_) | SessionKind::Shell => {
                if running {
                    if theme::ghost(ui, "Open").clicked() {
                        cx.dispatch(AppAction::ReturnToSession(id));
                    }
                    if theme::ghost_muted(ui, "Kill").clicked() {
                        cx.dispatch(AppAction::KillSession(id));
                    }
                } else {
                    if theme::ghost(ui, "Return").clicked() {
                        cx.dispatch(AppAction::ReturnToSession(id));
                    }
                    if theme::ghost_muted(ui, "Remove").clicked() {
                        cx.dispatch(AppAction::RemoveSession(id));
                    }
                }
            }
            SessionKind::Command | SessionKind::Service => {
                super::runs::actions(cx, ui, record, running);
            }
        }
        dismiss_ask(cx, ui, record.id);
    });
}

/// "Dismiss question" for a session that asked the owner something.
/// The question shows only while the pane runs, so its dismiss does too.
pub(super) fn dismiss_ask(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: RecordId) {
    let asking = cx.core.is_running(id) && cx.core.session(id).is_some_and(|s| s.asking.is_some());
    if asking
        && theme::ghost_muted(ui, DISMISS_ASK)
            .on_hover_text("Clear the session's question; the session is not told")
            .clicked()
    {
        cx.dispatch(AppAction::DismissAsk(id));
    }
}

/// The label of the button that clears a session's own question; not
/// "Dismiss", which takes a card off a rule set.
pub(super) const DISMISS_ASK: &str = "Dismiss question";

/// A live defined entry would come back on the next read, so Remove is
/// for the user's own records and orphans only.
pub(super) fn removable(record: &SessionRecord) -> bool {
    matches!(
        record.approval(),
        Approval::NotApplicable | Approval::Orphaned
    )
}

/// The dashed "+ New session" cell that ends the agents grid.
pub fn new_session_cell(ui: &mut Ui) -> bool {
    let p = theme::palette(ui);
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    let text = "+ New session";
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, text));
    if response.hovered() {
        ui.painter().rect_filled(rect, 2.0, p.text_alpha(0.04));
    }
    theme::dashed_rect(ui, rect.shrink(0.5), p.n400);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::new(14.0, egui::FontFamily::Proportional),
        p.accent_text,
    );
    response.clicked()
}

/// A pinned document: its name previews it; the buttons open it in the
/// default app or unpin it. `rel` is the path as stored (relative to the
/// project root), `path` the absolute one.
pub fn document_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: ProjectId, rel: &Path, path: &Path) {
    let p = theme::palette(ui);
    let mut open = false;
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.style_mut().interaction.selectable_labels = false;
            theme::kicker(ui, "Document", p.n600);
            open = ui
                .add(
                    egui::Label::new(
                        RichText::new(file_name(path)).text_style(theme::card_title()),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
            // The folder, when there is one; the name is above.
            if let Some(dir) = rel.parent().filter(|d| !d.as_os_str().is_empty()) {
                ui.add(
                    egui::Label::new(theme::mono_text(ui, dir.display().to_string())).truncate(),
                );
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    super::action_spacing(ui);
                    if theme::ghost(ui, "Open in app")
                        .on_hover_text("Open with the default app")
                        .clicked()
                    {
                        cx.dispatch(AppAction::OpenDocument(path.to_path_buf()));
                    }
                    if theme::ghost_muted(ui, "Unpin").clicked() {
                        cx.dispatch(AppAction::UnpinDocument(pid, rel.to_path_buf()));
                    }
                });
            });
        });
    if open {
        cx.dispatch(AppAction::ShowDocument(pid, path.to_path_buf()));
    }
}

/// What an agent card says under its title: the first line of the last
/// answer, else what the agent is doing, else the prompt it is on.
fn agent_excerpt(c: &Conversation) -> Option<String> {
    let turn = c.turns.last()?;
    let first_line = |text: &str| {
        text.lines()
            .map(|l| l.trim().trim_start_matches(['#', '-', '*', '>']).trim())
            .find(|l| !l.is_empty())
            .map(str::to_owned)
    };
    let text = first_line(&turn.final_text)
        .or_else(|| turn.activity.last().map(|a| a.line.clone()))
        .or_else(|| first_line(&turn.user).map(|u| format!("You: {u}")))?;
    let mut out: String = text.chars().take(160).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    Some(out)
}

/// The last line of a pane with something to read on it: a bare prompt
/// glyph is not worth a line, and the serif has no shape for it anyway.
fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| l.chars().any(char::is_alphanumeric))
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::ago_ms;

    #[test]
    fn a_relative_time_reads_in_its_largest_units() {
        let now = 10_000_000_000;
        let ago = |secs: u64| ago_ms(now - secs * 1000, now);
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(60), "1 min ago");
        assert_eq!(ago(59 * 60), "59 min ago");
        assert_eq!(ago(3600), "1 h ago");
        assert_eq!(ago(2 * 3600 + 14 * 60), "2 h 14 min ago");
        assert_eq!(ago(24 * 3600), "1 d ago");
        assert_eq!(ago(2 * 24 * 3600), "2 d ago");
        assert_eq!(ago_ms(now + 5_000, now), "just now", "a clock ahead");
        assert_eq!(ago_ms(0, now), "");
    }
}
