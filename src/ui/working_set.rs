//! The Working Set: the user's own grid of cards from any project,
//! sessions and files alike, each where they put it. Cards sit on a
//! grid of fixed units; the number of columns comes from the window,
//! and a card past the right edge is reached by scrolling.

use std::path::Path;

use egui::{RichText, Sense, Ui, UiBuilder, vec2};

use super::cards::{file_name, session_card};
use super::{DrawCtx, theme};
use crate::core::{AppAction, GridRect, PinTarget, PinnedItem};

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
    ui.label(RichText::new("Working Set").text_style(theme::h1()));
    let items: Vec<PinnedItem> = cx
        .core
        .working_set()
        .map(|s| s.items.clone())
        .unwrap_or_default();
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
    let columns = columns(ui.available_width());
    let width_units = items
        .iter()
        .map(|i| i.rect.x + i.rect.w)
        .fold(columns, u32::max);
    let height_units = items.iter().map(|i| i.rect.y + i.rect.h).max().unwrap_or(0);
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
            for item in &items {
                let cell = cell_rect(origin, item.rect);
                ui.scope_builder(UiBuilder::new().max_rect(cell), |ui| {
                    ui.set_clip_rect(cell.intersect(ui.clip_rect()));
                    card(cx, ui, item);
                });
            }
        });
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
                    if theme::ghost_muted(ui, "Remove")
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
