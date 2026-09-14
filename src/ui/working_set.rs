//! The Working Set: the user's own grid of cards from any project,
//! sessions and files alike, each where they put it. Cards sit on a
//! grid of fixed units; the number of columns comes from the window,
//! and a card past the right edge is reached by scrolling.

use std::path::Path;

use egui::{Pos2, RichText, Sense, Ui, UiBuilder, vec2};

use super::cards::{file_name, session_card};
use super::{DrawCtx, theme};
use crate::core::grid::{MIN_HEIGHT, MIN_WIDTH};
use crate::core::{AppAction, GridRect, PinTarget, PinnedItem};

/// Arrange mode: while on, cards are moved and resized instead of
/// used, and the one being dragged follows the pointer in whole units.
#[derive(Debug, Default)]
pub struct Arrange {
    pub on: bool,
    pub drag: Option<Drag>,
}

/// One drag in progress: what is dragged, where it started, and where
/// it would land if released now.
#[derive(Debug, Clone)]
pub struct Drag {
    pub target: PinTarget,
    pub from: GridRect,
    /// The corner handle resizes; anywhere else moves.
    pub resize: bool,
    pub start: Pos2,
    pub preview: GridRect,
}

/// The corner handle's side, in points.
const HANDLE: f32 = 16.0;

/// Where a drag that started at `from` lands after the pointer moved
/// by `delta` points: the same rectangle shifted (or grown) by the
/// nearest whole number of units, never past the left or top edge and
/// never below the minimum size.
#[must_use]
pub fn dragged_rect(from: GridRect, resize: bool, delta: egui::Vec2) -> GridRect {
    #[allow(clippy::cast_possible_truncation)]
    let (dx, dy) = (
        (delta.x / UNIT).round() as i64,
        (delta.y / UNIT).round() as i64,
    );
    let shift = |base: u32, by: i64, min: u32| -> u32 {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let v = (i64::from(base) + by).max(i64::from(min)) as u32;
        v
    };
    if resize {
        GridRect {
            w: shift(from.w, dx, MIN_WIDTH),
            h: shift(from.h, dy, MIN_HEIGHT),
            ..from
        }
    } else {
        GridRect {
            x: shift(from.x, dx, 0),
            y: shift(from.y, dy, 0),
            ..from
        }
    }
}

/// One grid unit in points, gap included: seven units make a board
/// card's 230 px.
pub const UNIT: f32 = 34.0;
/// The gap between neighbouring cards, taken off each card's right and
/// bottom edge.
const GAP_PX: f32 = 8.0;

/// How many whole units fit in `width` points.
#[must_use]
pub fn columns(width: f32) -> u32 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = ((width + GAP_PX) / UNIT).floor() as u32;
    n.max(crate::core::grid::MIN_WIDTH)
}

/// The points rectangle of a grid rectangle placed at `origin`.
#[must_use]
pub fn cell_rect(origin: egui::Pos2, rect: GridRect) -> egui::Rect {
    #[allow(clippy::cast_precision_loss)]
    let (x, y, w, h) = (rect.x as f32, rect.y as f32, rect.w as f32, rect.h as f32);
    egui::Rect::from_min_size(
        origin + vec2(x * UNIT, y * UNIT),
        vec2(w * UNIT - GAP_PX, h * UNIT - GAP_PX),
    )
}

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing = egui::vec2(10.0, 6.0);
    let mut items: Vec<PinnedItem> = cx
        .core
        .working_set()
        .map(|s| s.items.clone())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !items.is_empty() {
                let arranging = cx.state.arrange.on;
                let button = if arranging {
                    theme::primary(ui, "Done")
                } else {
                    theme::secondary(ui, "Arrange")
                };
                if button
                    .on_hover_text("Drag cards to move them; drag a corner to resize")
                    .clicked()
                {
                    cx.state.arrange.on = !arranging;
                    cx.state.arrange.drag = None;
                }
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.label(RichText::new("Working Set").text_style(theme::h1()));
            });
        });
    });
    ui.label(theme::meta_text(
        ui,
        match items.len() {
            0 => "Nothing here yet.".to_owned(),
            1 => "1 card".to_owned(),
            n => format!("{n} cards"),
        },
    ));
    if items.is_empty() {
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Add a session from its card's menu or its header, and a file from the file tree's menu.",
            )
            .color(p.n700),
        );
        return;
    }
    ui.add_space(8.0);
    let arranging = cx.state.arrange.on;
    // The card being dragged is drawn where it would land, so it snaps
    // along under the pointer; the record itself moves on release.
    if let Some(drag) = &cx.state.arrange.drag
        && let Some(item) = items.iter_mut().find(|i| i.target == drag.target)
    {
        item.rect = drag.preview;
    }
    let columns = columns(ui.available_width());
    let width_units = items
        .iter()
        .map(|i| i.rect.x + i.rect.w)
        .fold(columns, u32::max);
    // Room below the last card while arranging, to drag one down into.
    let extra = if arranging { 8 } else { 0 };
    let height_units = items.iter().map(|i| i.rect.y + i.rect.h).max().unwrap_or(0) + extra;
    egui::ScrollArea::both()
        .id_salt("working-set")
        .auto_shrink(false)
        .show(ui, |ui| {
            #[allow(clippy::cast_precision_loss)]
            let total = vec2(
                width_units as f32 * UNIT - GAP_PX,
                height_units as f32 * UNIT - GAP_PX,
            );
            let (rect, _) = ui.allocate_exact_size(total, Sense::hover());
            let origin = rect.min;
            if arranging {
                let cells: Vec<egui::Rect> =
                    items.iter().map(|i| cell_rect(origin, i.rect)).collect();
                grid_dots(ui, rect, width_units, height_units, &cells);
            }
            for item in &items {
                let cell = cell_rect(origin, item.rect);
                ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                    ui.set_clip_rect(cell.intersect(ui.clip_rect()));
                    if arranging {
                        ui.disable();
                    }
                    card(cx, ui, item);
                });
                if arranging {
                    arrange_handles(cx, ui, item, cell);
                }
            }
        });
}

/// A dot at every unit corner not under a card, so the grid a card
/// snaps to is visible (the dimmed cards would let dots show through).
fn grid_dots(ui: &Ui, rect: egui::Rect, width_units: u32, height_units: u32, cells: &[egui::Rect]) {
    let p = theme::palette(ui);
    let painter = ui.painter();
    for y in 0..=height_units {
        for x in 0..=width_units {
            #[allow(clippy::cast_precision_loss)]
            let at = rect.min + vec2(x as f32 * UNIT, y as f32 * UNIT)
                - vec2(GAP_PX / 2.0, GAP_PX / 2.0);
            if cells.iter().any(|c| c.contains(at)) {
                continue;
            }
            painter.circle_filled(at, 1.0, p.n300);
        }
    }
}

/// The move and resize senses over one card while arranging: the whole
/// cell moves it, the bottom-right corner resizes it. The outline shows
/// the drop: cyan where it fits, magenta where it would land on
/// another card and be refused.
fn arrange_handles(cx: &mut DrawCtx<'_>, ui: &mut Ui, item: &PinnedItem, cell: egui::Rect) {
    let p = theme::palette(ui);
    let id = ui.id().with(("arrange", &item.target));
    let handle = egui::Rect::from_min_size(cell.max - vec2(HANDLE, HANDLE), vec2(HANDLE, HANDLE));
    let body = ui
        .interact(cell, id.with("move"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::Grab);
    let corner = ui
        .interact(handle, id.with("resize"), Sense::drag())
        .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
    let dragging = cx
        .state
        .arrange
        .drag
        .as_ref()
        .is_some_and(|d| d.target == item.target);
    for (response, resize) in [(&corner, true), (&body, false)] {
        if response.drag_started()
            && let Some(start) = response.interact_pointer_pos()
        {
            cx.state.arrange.drag = Some(Drag {
                target: item.target.clone(),
                from: item.rect,
                resize,
                start,
                preview: item.rect,
            });
        }
        if response.dragged()
            && let Some(at) = response.interact_pointer_pos()
            && let Some(drag) = cx.state.arrange.drag.as_mut()
            && drag.target == item.target
        {
            drag.preview = dragged_rect(drag.from, drag.resize, at - drag.start);
        }
        if response.drag_stopped()
            && let Some(drag) = cx.state.arrange.drag.take()
        {
            cx.dispatch(AppAction::PlacePin {
                target: drag.target,
                rect: drag.preview,
            });
        }
    }
    let fits = cx
        .core
        .working_set()
        .is_some_and(|s| crate::core::grid::fits(&s.items, &item.target, item.rect));
    let color = if !fits {
        p.accent_2
    } else if dragging || body.hovered() || corner.hovered() {
        p.accent
    } else {
        p.n300
    };
    let painter = ui.painter();
    painter.rect_stroke(
        cell,
        egui::CornerRadius::same(2),
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(handle, egui::CornerRadius::same(2), color);
}

fn card(cx: &mut DrawCtx<'_>, ui: &mut Ui, item: &PinnedItem) {
    match &item.target {
        PinTarget::Session(id) => {
            if let Some(record) = cx.core.session(*id) {
                session_card(cx, ui, record);
            }
        }
        PinTarget::File(pid, rel) => file_card(cx, ui, *pid, rel),
    }
}

/// A file on the working set: its name, folder, and project, with
/// Preview, Open in app, and Remove.
fn file_card(cx: &mut DrawCtx<'_>, ui: &mut Ui, pid: crate::core::ProjectId, rel: &Path) {
    let p = theme::palette(ui);
    let Some(project) = cx.core.workspace(pid).map(|w| &w.project) else {
        return;
    };
    let path = project.root.join(rel);
    let target = PinTarget::File(pid, rel.to_path_buf());
    let mut open = false;
    theme::surface(ui)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
            ui.style_mut().interaction.selectable_labels = false;
            theme::kicker(ui, &format!("File · {}", project.name), p.n600);
            open = ui
                .add(
                    egui::Label::new(
                        RichText::new(file_name(&path)).text_style(theme::card_title()),
                    )
                    .truncate()
                    .sense(Sense::click()),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked();
            if let Some(dir) = rel.parent().filter(|d| !d.as_os_str().is_empty()) {
                ui.add(
                    egui::Label::new(theme::mono_text(ui, dir.display().to_string())).truncate(),
                );
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 14.0;
                    ui.spacing_mut().button_padding = vec2(0.0, 4.0);
                    if theme::ghost(ui, "Open in app").clicked() {
                        cx.dispatch(AppAction::OpenDocument(path.clone()));
                    }
                    if theme::ghost_muted(ui, "Take off")
                        .on_hover_text("Take it off the working set")
                        .clicked()
                    {
                        cx.dispatch(AppAction::RemoveFromWorkingSet(target.clone()));
                    }
                });
            });
        });
    if open {
        cx.dispatch(AppAction::ShowDocument(pid, path));
    }
}

/// The menu item that puts `target` on the working set or takes it
/// off, for card menus and context menus.
pub fn menu_item(cx: &mut DrawCtx<'_>, ui: &mut Ui, target: PinTarget) -> bool {
    let on = cx.core.in_working_set(&target);
    let label = if on {
        "Remove from working set"
    } else {
        "Add to working set"
    };
    if ui.button(label).clicked() {
        cx.dispatch(if on {
            AppAction::RemoveFromWorkingSet(target)
        } else {
            AppAction::AddToWorkingSet {
                target,
                columns: cx.state.working_set_columns,
            }
        });
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_snaps_to_whole_units_and_respects_the_edges_and_minimums() {
        let from = GridRect {
            x: 2,
            y: 1,
            w: 10,
            h: 8,
        };
        // Less than half a unit is nothing; more rounds to the unit.
        assert_eq!(dragged_rect(from, false, vec2(10.0, 0.0)), from);
        assert_eq!(
            dragged_rect(from, false, vec2(UNIT * 2.6, -UNIT * 1.4)),
            GridRect { x: 5, y: 0, ..from }
        );
        // Never past the top-left corner.
        assert_eq!(
            dragged_rect(from, false, vec2(-UNIT * 9.0, -UNIT * 9.0)),
            GridRect { x: 0, y: 0, ..from }
        );
        // Resizing keeps the corner and never goes under the minimums.
        assert_eq!(
            dragged_rect(from, true, vec2(UNIT * 2.0, UNIT)),
            GridRect {
                w: 12,
                h: 9,
                ..from
            }
        );
        assert_eq!(
            dragged_rect(from, true, vec2(-UNIT * 20.0, -UNIT * 20.0)),
            GridRect {
                w: MIN_WIDTH,
                h: MIN_HEIGHT,
                ..from
            }
        );
    }

    #[test]
    fn seven_units_are_a_board_card_and_columns_come_from_the_width() {
        let seven = cell_rect(
            egui::Pos2::ZERO,
            GridRect {
                x: 0,
                y: 0,
                w: 7,
                h: 5,
            },
        );
        assert!((seven.width() - 230.0).abs() < 0.01);
        assert_eq!(columns(1000.0), 29);
        assert_eq!(columns(10.0), crate::core::grid::MIN_WIDTH);
    }
}
