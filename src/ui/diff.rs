//! One changed file's diff under its row on the ticket page's Changes
//! tab: hunks highlighted and unwrapped, changed rows tinted and their
//! changed words marked, long unchanged stretches folded. The core
//! holds the diff and says what folds; this only draws.

use std::ops::Range;
use std::path::Path;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, Ui};

use super::ticket::LaneChanges;
use super::{DrawCtx, theme};
use crate::core::AppAction;
use crate::core::diff::{Run, runs};
use crate::ports::changes::{DiffBody, DiffLine, FileDiff, FileStat, Hunk, LineKind};

/// The action that reads `file`'s diff over `lane`'s range.
fn read(lane: &LaneChanges<'_>, file: &FileStat) -> AppAction {
    AppAction::ReadFileDiff {
        ticket: lane.ticket.id.clone(),
        lane: lane.lane.to_owned(),
        path: file.path.clone(),
        old_path: file.old_path.clone(),
        range: lane.range.clone(),
    }
}

/// Ask for the diff when the core says one is due, then draw what is
/// held: the last answer stays up while a re-read is out.
pub(super) fn file_diff(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    lane: &LaneChanges<'_>,
    file: &FileStat,
) {
    let (ticket, path) = (lane.ticket.id.as_str(), file.path.as_str());
    if cx.core.file_diff_due(ticket, lane.lane, path, lane.range) {
        cx.dispatch(read(lane, file));
    }
    // A copy of the shared reference, so the held diff stays borrowed
    // from the core while `cx` is borrowed mutably to dispatch.
    let core = cx.core;
    let held = core
        .file_diff(ticket, lane.lane, path)
        .and_then(|r| r.last.as_ref())
        .map(|(_, result)| result);
    match held {
        None => {
            ui.label(theme::meta_text(ui, "Reading…"));
        }
        Some(Err(e)) => {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::meta_text(ui, format!("git: {e}")));
                if theme::ghost(ui, "Retry").clicked() {
                    cx.dispatch(read(lane, file));
                }
            });
        }
        Some(Ok(diff)) => body(cx, ui, lane, file, diff),
    }
}

/// The hunks, or a note in their place; the file's row above already
/// offers Open and Reveal.
fn body(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    lane: &LaneChanges<'_>,
    file: &FileStat,
    diff: &FileDiff,
) {
    let note = match &diff.body {
        DiffBody::Hunks(hunks) => {
            for (i, hunk) in hunks.iter().enumerate() {
                if hunks.len() > 1 {
                    ui.label(theme::mono_text(
                        ui,
                        format!("@@ -{} +{} @@", hunk.old_start, hunk.new_start),
                    ));
                }
                egui::ScrollArea::horizontal()
                    .id_salt(("diff", &lane.ticket.id, lane.lane, &file.path, i))
                    .auto_shrink([false, true])
                    .show(ui, |ui| draw_hunk(cx, ui, lane, file, hunk));
            }
            return;
        }
        DiffBody::Binary => "Binary file.".to_owned(),
        DiffBody::TooLarge { lines } => format!("Too large to show here ({lines} lines)."),
        DiffBody::Empty if file.old_path.is_some() => "Renamed, no line changes.".to_owned(),
        DiffBody::Empty => "No line changes.".to_owned(),
    };
    ui.label(theme::meta_text(ui, note));
}

/// A hunk as its runs: folded stretches as one button each, unless the
/// owner expanded them, and the rest as blocks of lines. A fold is
/// remembered by its first line's new-side number, so it stays with
/// those lines when a re-read moves the hunks.
fn draw_hunk(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    lane: &LaneChanges<'_>,
    file: &FileStat,
    hunk: &Hunk,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    for run in runs(&hunk.lines) {
        let lines = match run {
            Run::Lines(r) => r,
            Run::Folded(r) => {
                let key = (
                    lane.ticket.id.clone(),
                    lane.lane.to_owned(),
                    file.path.clone(),
                    hunk.lines[r.start].new_no,
                );
                if !cx.state.diff_unfolded.contains(&key) {
                    let text = format!("⋯ {} unchanged lines", r.len());
                    if theme::ghost_muted(ui, &text).clicked() {
                        cx.state.diff_unfolded.insert(key);
                    }
                    continue;
                }
                r
            }
        };
        draw_lines(cx, ui, lane, file, &hunk.lines[lines]);
    }
}

/// Lines as a gutter of line numbers and a block of highlighted code,
/// changed rows tinted full width behind both. A new-side number opens
/// the file at that line while the tree stands.
fn draw_lines(
    cx: &mut DrawCtx<'_>,
    ui: &mut Ui,
    lane: &LaneChanges<'_>,
    file: &FileStat,
    lines: &[DiffLine],
) {
    let p = theme::palette(ui);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let gutter: String = lines
        .iter()
        .map(|l| {
            let no = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
            let sign = match l.kind {
                LineKind::Context => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
            };
            format!("{:>5} {:>5} {sign}", no(l.old_no), no(l.new_no))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let gutter = ui.painter().layout_no_wrap(gutter, font, p.n600);
    let code = code_job(ui, Path::new(&file.path), lines);
    let code = ui.painter().layout_job(code);
    // Reserved before the text so the tints are painted under it.
    let tints = ui.painter().add(egui::Shape::Noop);
    let (g, c) = ui
        .horizontal(|ui| {
            let sense = if lane.tree.is_some() {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            };
            let g = ui.add(
                egui::Label::new(gutter.clone())
                    .selectable(false)
                    .sense(sense),
            );
            let c = ui.add(
                egui::Label::new(code)
                    .selectable(true)
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
            (g, c)
        })
        .inner;
    // The gutter has one row per line, in the code's font, so its rows
    // say where each line is on screen.
    let rows: Vec<egui::Rangef> = gutter
        .rows
        .iter()
        .map(|r| r.rect().translate(g.rect.min.to_vec2()).y_range())
        .collect();
    let right = c.rect.right().max(ui.max_rect().right());
    let shapes = lines
        .iter()
        .zip(&rows)
        .filter_map(|(l, y)| {
            let tint = match l.kind {
                LineKind::Context => return None,
                LineKind::Added => p.diff_add,
                LineKind::Removed => p.diff_del,
            };
            let rect = egui::Rect::from_x_y_ranges(g.rect.left()..=right, *y);
            Some(egui::Shape::rect_filled(rect, 0.0, tint))
        })
        .collect::<Vec<_>>();
    ui.painter().set(tints, egui::Shape::Vec(shapes));
    let Some(tree) = lane.tree else {
        return;
    };
    let line_at = |y: f32| {
        let n = rows.iter().position(|r| r.contains(y))?;
        lines.get(n).and_then(|l| l.new_no)
    };
    if g.hover_pos().and_then(|pos| line_at(pos.y)).is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if g.clicked()
        && let Some(line) = g.interact_pointer_pos().and_then(|pos| line_at(pos.y))
    {
        cx.dispatch(AppAction::ShowFileAt {
            path: tree.join(&file.path),
            line,
            project: lane.ticket.root_project.clone(),
        });
    }
}

/// The lines joined and highlighted once, then cut at each changed
/// word to give it its background, with a note after a line that had
/// no newline. Unwrapped, so each line is one row beside the gutter.
fn code_job(ui: &Ui, path: &Path, lines: &[DiffLine]) -> LayoutJob {
    let p = theme::palette(ui);
    let mut text = String::new();
    let mut marks: Vec<(Range<usize>, Color32)> = Vec::new();
    let mut notes = Vec::new();
    for (n, l) in lines.iter().enumerate() {
        if n > 0 {
            text.push('\n');
        }
        let start = text.len();
        text.push_str(&l.text);
        let color = match l.kind {
            LineKind::Added => p.diff_add_word,
            _ => p.diff_del_word,
        };
        marks.extend(
            l.marks
                .iter()
                .map(|m| (start + m.start..start + m.end, color)),
        );
        if l.no_newline {
            notes.push(text.len());
        }
    }
    let (code_theme, lang) = super::document::highlighting_for(ui, path);
    let job =
        egui_extras::syntax_highlighting::highlight(ui.ctx(), ui.style(), &code_theme, &text, lang);
    let note = TextFormat {
        font_id: egui::TextStyle::Monospace.resolve(ui.style()),
        color: p.n600,
        ..TextFormat::default()
    };
    let mut job = decorate(job, &marks, &notes, &note);
    job.wrap.max_width = f32::INFINITY;
    job
}

/// `job` with its sections cut at the edges of `marks`, each piece
/// inside a mark given its background, and the note "no newline at end
/// of file" put at each offset of `notes`. Both are sorted and do not
/// overlap, since they come line by line.
fn decorate(
    mut job: LayoutJob,
    marks: &[(Range<usize>, Color32)],
    notes: &[usize],
    note: &TextFormat,
) -> LayoutJob {
    const NOTE: &str = " no newline at end of file";
    let text = std::mem::take(&mut job.text);
    let sections = std::mem::take(&mut job.sections);
    let mut cuts: Vec<usize> = marks
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .chain(notes.iter().copied())
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    if notes.first() == Some(&0) {
        job.append(NOTE, 0.0, note.clone());
    }
    for section in sections {
        let (mut at, end) = (section.byte_range.start.0, section.byte_range.end.0);
        while at < end {
            let next = cuts
                .get(cuts.partition_point(|&c| c <= at))
                .copied()
                .filter(|&c| c < end)
                .unwrap_or(end);
            let mut format = section.format.clone();
            let mark = marks.partition_point(|(r, _)| r.end <= at);
            if let Some((r, color)) = marks.get(mark)
                && r.start <= at
            {
                format.background = *color;
            }
            job.append(&text[at..next], section.leading_space, format);
            if notes.binary_search(&next).is_ok() {
                job.append(NOTE, 0.0, note.clone());
            }
            at = next;
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> LayoutJob {
        LayoutJob::single_section(text.to_owned(), TextFormat::default())
    }

    #[test]
    fn marks_and_notes_cut_the_sections() {
        let red = Color32::RED;
        let job = decorate(
            plain("let y = 1;\nz"),
            &[(4..5, red)],
            &[12],
            &TextFormat {
                color: Color32::BLUE,
                ..TextFormat::default()
            },
        );
        let pieces: Vec<(&str, Color32)> = job
            .sections
            .iter()
            .map(|s| {
                let r = s.byte_range.start.0..s.byte_range.end.0;
                (&job.text[r], s.format.background)
            })
            .collect();
        assert_eq!(
            pieces,
            vec![
                ("let ", Color32::TRANSPARENT),
                ("y", red),
                (" = 1;\nz", Color32::TRANSPARENT),
                (" no newline at end of file", Color32::TRANSPARENT),
            ]
        );
    }
}
