//! The app's version and its update, in the Settings menu and as one
//! item on the rail while there is something to click.

use egui::{RichText, Ui};

use super::{DrawCtx, theme};
use crate::core::AppCore;
use crate::core::action::AppAction;
use crate::core::updates::UpdateState;

/// The version line and the update's state with its one button.
pub fn settings_rows(cx: &mut DrawCtx<'_>, ui: &mut Ui) {
    let p = theme::palette(ui);
    ui.label(
        RichText::new(format!("Switchboard {}", cx.core.version()))
            .text_style(theme::meta())
            .color(p.n700),
    );
    let line = |ui: &mut Ui, text: &str| {
        ui.label(theme::meta_text(ui, text));
    };
    match cx.core.update() {
        UpdateState::Idle => {
            if theme::ghost(ui, "Check for updates").clicked() {
                cx.dispatch(AppAction::CheckForUpdate);
            }
        }
        UpdateState::Checking => line(ui, "Checking for updates…"),
        UpdateState::UpToDate => {
            line(ui, "Up to date");
            if theme::ghost(ui, "Check again").clicked() {
                cx.dispatch(AppAction::CheckForUpdate);
            }
        }
        UpdateState::Available(release) => {
            line(ui, &format!("{} is available", release.version));
            if theme::ghost(ui, &format!("Download {}", release.version))
                .on_hover_text(notes(&release.notes))
                .clicked()
            {
                cx.dispatch(AppAction::DownloadUpdate);
            }
        }
        UpdateState::Downloading(release) => {
            line(ui, &format!("Downloading {}…", release.version));
        }
        UpdateState::Ready { release, .. } => {
            line(ui, &format!("{} is ready", release.version));
            if theme::ghost(ui, "Restart to update")
                .on_hover_text("Quits, swaps the bundle in ~/Applications and relaunches")
                .clicked()
            {
                cx.dispatch(AppAction::InstallUpdate);
            }
        }
        UpdateState::Failed { reason, release } => {
            ui.label(
                RichText::new(reason)
                    .text_style(theme::meta())
                    .color(p.accent_2_text),
            );
            let (label, action) = if release.is_some() {
                ("Try the download again", AppAction::DownloadUpdate)
            } else {
                ("Check again", AppAction::CheckForUpdate)
            };
            if theme::ghost(ui, label).clicked() {
                cx.dispatch(action);
            }
        }
    }
}

/// The rail's item while an update waits on a click: its label, the
/// hover text and the action. Nothing while there is nothing to do.
#[must_use]
pub fn rail_item(core: &AppCore) -> Option<(&'static str, String, AppAction)> {
    match core.update() {
        UpdateState::Available(release) => Some((
            "Update",
            format!("Switchboard {} is available; download it", release.version),
            AppAction::DownloadUpdate,
        )),
        UpdateState::Ready { release, .. } => Some((
            "Restart to update",
            format!(
                "Switchboard {} is ready; quit, swap and relaunch",
                release.version
            ),
            AppAction::InstallUpdate,
        )),
        _ => None,
    }
}

/// The release notes as hover text: the first lines, cut short.
fn notes(text: &str) -> String {
    let mut out: String = text.lines().take(12).collect::<Vec<_>>().join("\n");
    if out.len() > 600 {
        out.truncate(600);
        out.push('…');
    }
    out
}
