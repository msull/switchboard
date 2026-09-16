//! Notes tab of the side panel: the session's notes, edited in place.
//! The draft is transient and follows the session on screen; every
//! change goes to the core, which saves the record.

use egui::{Margin, Ui};

use super::{DrawCtx, theme};
use crate::core::{AppAction, RecordId};

/// The accessible name of the field; the tab itself is "Notes".
pub const FIELD_NAME: &str = "Session notes";

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut Ui, id: RecordId) {
    let Some(record) = cx.core.session(id) else {
        return;
    };
    // A different record on screen means a fresh draft from its notes.
    let stale = cx
        .state
        .notes_draft
        .as_ref()
        .is_none_or(|(drafted, _)| *drafted != id);
    if stale {
        cx.state.notes_draft = Some((id, record.notes.clone()));
    }
    let Some((_, draft)) = cx.state.notes_draft.as_mut() else {
        return;
    };
    let p = theme::palette(ui);
    let response = ui.add_sized(
        ui.available_size(),
        egui::TextEdit::multiline(draft)
            .hint_text("Notes about this session: what it is for, where it was left")
            .font(theme::meta())
            .text_color(p.n700)
            .background_color(p.surface)
            .margin(Margin::symmetric(10, 8)),
    );
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_label(FIELD_NAME));
    let changed = response.changed().then(|| draft.clone());
    if let Some(text) = changed {
        cx.dispatch(AppAction::SetSessionNotes(id, text));
    }
}
