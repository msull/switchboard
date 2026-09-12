//! Markdown for answers and previews. Prose goes through
//! `egui_commonmark`; tables are laid out here. The viewer puts a table
//! in an `egui::Grid` and lets every cell wrap at the full width, so
//! columns land on top of each other and a long cell takes the whole
//! line. Here each column gets a width from its content, the way a
//! browser sizes a table, and cells wrap inside it.

use egui::{Frame, Layout, Margin, Shape, Ui};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::theme;

/// A piece of a Markdown document: text for the viewer, or one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment<'a> {
    Prose(&'a str),
    Table(Table<'a>),
}

/// The cells of one table as Markdown source, inline marks included.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table<'a> {
    pub header: Vec<&'a str>,
    pub rows: Vec<Vec<&'a str>>,
}

impl Table<'_> {
    fn columns(&self) -> usize {
        self.rows
            .iter()
            .map(Vec::len)
            .fold(self.header.len(), usize::max)
    }
}

/// Cut `text` into prose and its top-level tables, in order. A table
/// inside a list or a quote stays with the prose: cutting it out would
/// break the block around it.
#[must_use]
pub fn split(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut prose_from = 0;
    let mut depth = 0usize;
    let mut table: Option<(usize, usize, Table<'_>)> = None;
    let mut in_head = false;
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES);
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(Tag::Table(_)) if depth == 0 => {
                table = Some((range.start, range.end, Table::default()));
                depth += 1;
            }
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Start(Tag::TableRow) => {
                if let Some((_, _, t)) = table.as_mut() {
                    t.rows.push(Vec::new());
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some((_, _, t)) = table.as_mut() {
                    let cell = text[range.clone()].trim();
                    if in_head {
                        t.header.push(cell);
                    } else if let Some(row) = t.rows.last_mut() {
                        row.push(cell);
                    }
                }
                depth += 1;
            }
            Event::Start(_) => depth += 1,
            Event::End(TagEnd::Table) => {
                depth = depth.saturating_sub(1);
                if let Some((start, end, t)) = table.take() {
                    let before = &text[prose_from..start];
                    if !before.trim().is_empty() {
                        out.push(Segment::Prose(before));
                    }
                    out.push(Segment::Table(t));
                    prose_from = end;
                }
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    let rest = &text[prose_from..];
    if !rest.trim().is_empty() || out.is_empty() {
        out.push(Segment::Prose(rest));
    }
    out
}

/// Column widths for a table whose columns want `max[i]` unwrapped and
/// can shrink to `min[i]` (their longest word). Everything fits: each
/// column gets what it wants. Otherwise the slack above the minimums is
/// shared in proportion to how much each column would like to grow, so
/// a column of short values stays narrow. Below the minimums the table
/// is wider than the view and scrolls.
#[must_use]
pub fn column_widths(min: &[f32], max: &[f32], available: f32) -> Vec<f32> {
    let want: f32 = max.iter().sum();
    if want <= available {
        return max.to_vec();
    }
    let need: f32 = min.iter().sum();
    if need >= available {
        return min.to_vec();
    }
    let slack = available - need;
    let growth: f32 = min.iter().zip(max).map(|(lo, hi)| hi - lo).sum();
    min.iter()
        .zip(max)
        .map(|(lo, hi)| {
            if growth > 0.0 {
                lo + slack * (hi - lo) / growth
            } else {
                *lo
            }
        })
        .collect()
}

/// Draw `text`: prose through the viewer, tables through [`table`].
pub fn show(ui: &mut Ui, cache: &mut CommonMarkCache, text: &str) {
    for (i, segment) in split(text).into_iter().enumerate() {
        ui.push_id(i, |ui| match segment {
            Segment::Prose(prose) => {
                CommonMarkViewer::new().show(ui, cache, prose);
            }
            Segment::Table(t) => table(ui, cache, &t),
        });
    }
}

/// Space on each side of a cell's text.
const CELL_PAD: f32 = 8.0;

/// The width the cell's text takes on one line, and the width of its
/// widest word, both in the given font. Inline marks are dropped and a
/// code span is measured in the monospace font, which is what the
/// viewer will draw.
fn measure(ui: &Ui, cell: &str, font: &egui::FontId) -> (f32, f32) {
    let mono = egui::TextStyle::Monospace.resolve(ui.style());
    let width = |text: &str, font: &egui::FontId| {
        ui.painter()
            .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::BLACK)
            .size()
            .x
    };
    let mut max = 0.0_f32;
    let mut min = 0.0_f32;
    for (i, span) in cell.split('`').enumerate() {
        let (text, font) = if i % 2 == 1 {
            (span.to_owned(), &mono)
        } else {
            (plain(span), font)
        };
        if text.is_empty() {
            continue;
        }
        max += width(&text, font);
        for word in text.split_whitespace() {
            min = min.max(width(word, font));
        }
    }
    (min.min(max), max)
}

/// `cell` without the marks the viewer would not draw: emphasis stars,
/// escapes, and the target of a link.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' if chars.peek() == Some(&c) => {
                chars.next();
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '[' => out.push(' '),
            ']' if chars.peek() == Some(&'(') => {
                for next in chars.by_ref() {
                    if next == ')' {
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// One table: a bold header row on a tinted ground, a hairline under
/// every row, columns sized by [`column_widths`], cells wrapped inside
/// them by the viewer. Alignment marks in the source are ignored.
fn table(ui: &mut Ui, cache: &mut CommonMarkCache, t: &Table<'_>) {
    let columns = t.columns();
    if columns == 0 {
        return;
    }
    let body_font = egui::TextStyle::Body.resolve(ui.style());
    let head_font = theme::strong().resolve(ui.style());
    let mut min = vec![0.0_f32; columns];
    let mut max = vec![0.0_f32; columns];
    let mut note = |c: usize, cell: &str, font: &egui::FontId| {
        let (lo, hi) = measure(ui, cell, font);
        min[c] = min[c].max(lo);
        max[c] = max[c].max(hi);
    };
    for (c, cell) in t.header.iter().enumerate() {
        note(c, cell, &head_font);
    }
    for row in &t.rows {
        for (c, cell) in row.iter().enumerate() {
            note(c, cell, &body_font);
        }
    }
    // A hair of slack: a galley measured here and laid out in the cell
    // can differ by a rounding, and a word that just does not fit wraps.
    for hi in &mut max {
        *hi += 2.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let padding = columns as f32 * 2.0 * CELL_PAD;
    let widths = column_widths(&min, &max, ui.available_width() - padding);
    let total: f32 = widths.iter().map(|w| w + 2.0 * CELL_PAD).sum();

    ui.add_space(4.0);
    if !t.header.is_empty() {
        row(ui, cache, &widths, total, &t.header, Some(&head_font));
    }
    for cells in &t.rows {
        row(ui, cache, &widths, total, cells, None);
    }
    ui.add_space(4.0);
}

fn row(
    ui: &mut Ui,
    cache: &mut CommonMarkCache,
    widths: &[f32],
    total: f32,
    cells: &[&str],
    head_font: Option<&egui::FontId>,
) {
    let p = theme::palette(ui);
    // The ground is painted once the row's height is known: a shape
    // reserved now and filled in after the cells are drawn.
    let ground = ui.painter().add(Shape::Noop);
    let response = ui
        .horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (c, width) in widths.iter().enumerate() {
                let cell = cells.get(c).copied().unwrap_or("");
                ui.allocate_ui_with_layout(
                    egui::vec2(width + 2.0 * CELL_PAD, 0.0),
                    Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(width + 2.0 * CELL_PAD);
                        if let Some(font) = head_font {
                            ui.style_mut()
                                .text_styles
                                .insert(egui::TextStyle::Body, font.clone());
                        }
                        Frame::new()
                            .inner_margin(Margin::symmetric(8, 5))
                            .show(ui, |ui| {
                                ui.set_width(*width);
                                if !cell.is_empty() {
                                    ui.push_id(c, |ui| {
                                        CommonMarkViewer::new().show(ui, cache, cell);
                                    });
                                }
                            });
                    },
                );
            }
        })
        .response;
    let mut rect = response.rect;
    rect.set_width(total);
    if head_font.is_some() {
        ui.painter().set(
            ground,
            Shape::rect_filled(rect, egui::CornerRadius::same(2), p.n300),
        );
    }
    ui.painter()
        .hline(rect.x_range(), rect.bottom(), p.hairline());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_prose_around_a_table_and_keeps_cell_source() {
        let text = "Intro line.\n\n| Name | Value |\n|---|---|\n| `a` | **one** \\| two |\n| b | |\n\nAfter.\n";
        let segments = split(text);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0], Segment::Prose("Intro line.\n\n"));
        let Segment::Table(t) = &segments[1] else {
            panic!("{segments:?}");
        };
        assert_eq!(t.header, vec!["Name", "Value"]);
        assert_eq!(t.rows, vec![vec!["`a`", "**one** \\| two"], vec!["b", ""]]);
        assert_eq!(segments[2], Segment::Prose("\nAfter.\n"));
    }

    #[test]
    fn a_table_in_a_list_or_a_code_fence_is_left_to_the_viewer() {
        let listed = "- item\n\n  | a | b |\n  |---|---|\n  | 1 | 2 |\n";
        assert_eq!(split(listed), vec![Segment::Prose(listed)]);
        let fenced = "```\n| a | b |\n|---|---|\n```\n";
        assert_eq!(split(fenced), vec![Segment::Prose(fenced)]);
        assert_eq!(split(""), vec![Segment::Prose("")]);
    }

    #[test]
    fn two_tables_back_to_back() {
        let text = "| a |\n|---|\n| 1 |\n\n| b |\n|---|\n| 2 |\n";
        let segments = split(text);
        assert_eq!(segments.len(), 2);
        assert!(matches!(segments[0], Segment::Table(_)));
        assert!(matches!(segments[1], Segment::Table(_)));
    }

    #[test]
    fn widths_fit_then_share_then_scroll() {
        // Room for everything: columns take what they want.
        assert_eq!(
            column_widths(&[10.0, 10.0], &[50.0, 100.0], 200.0),
            vec![50.0, 100.0]
        );
        // Not enough for all: slack above the minimums goes to the
        // column that would grow the most; the short one stays short.
        let w = column_widths(&[20.0, 20.0], &[30.0, 190.0], 100.0);
        assert!(
            (w[0] - 23.33).abs() < 0.01 && (w[1] - 76.67).abs() < 0.01,
            "{w:?}"
        );
        assert!((w.iter().sum::<f32>() - 100.0).abs() < 0.01);
        // Even the longest words do not fit: minimums, and the table
        // scrolls sideways.
        assert_eq!(
            column_widths(&[60.0, 60.0], &[80.0, 80.0], 100.0),
            vec![60.0, 60.0]
        );
    }

    #[test]
    fn plain_drops_inline_marks() {
        assert_eq!(plain("**bold** and *it*"), "bold and *it*");
        assert_eq!(plain("a \\| b"), "a | b");
        assert_eq!(plain("[link](http://x)"), " link");
    }
}
