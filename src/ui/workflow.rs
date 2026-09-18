//! A workflow run: its state and rounds. The full page (plan versions,
//! feedback beside response, controls) is the next step; this shows
//! the run's state so the view exists.

use crate::core::{RunState, WorkflowId};
use crate::ui::{DrawCtx, theme};

pub fn show(cx: &mut DrawCtx<'_>, ui: &mut egui::Ui, id: WorkflowId) {
    let p = theme::palette(ui);
    let Some(run) = cx.core.workflow(id) else {
        ui.label("This review no longer exists.");
        return;
    };
    theme::kicker(ui, "Plan review", p.n600);
    ui.heading(
        run.plan
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plan"),
    );
    ui.label(format!(
        "Round {} · {}",
        run.rounds.len(),
        run.state.label()
    ));
    if let RunState::Paused(why) = &run.state {
        ui.label(why);
    }
}
