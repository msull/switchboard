//! Dispatch as the app shows it: the runner's last status, the
//! artifacts read for the ticket page, and the console, a shell session
//! of the app's own where `dispatch` commands are typed. The core holds
//! views from Dispatch's port and never its records; a status is data
//! that arrived, a decision answered is a call the app runs, and the
//! reply is one more action.

use std::collections::HashMap;
use std::path::PathBuf;

use super::action::{AppAction, AppCore, Clock, Effect, Out, View};
use super::model::{Launch, RecordId, SessionKind, Space, SpaceId};
use crate::ports::dispatch::{Body, DecisionView, Reply, Status, TicketView};

/// The space and project the console lives in, and its name.
pub const CONSOLE_SPACE: &str = "Dispatch";
pub const CONSOLE_NAME: &str = "console";

#[derive(Debug, Default)]
pub struct DispatchState {
    /// The last status answered; kept when the runner goes away, so the
    /// page still reads, marked stale.
    pub status: Status,
    /// A runner answered the last poll.
    pub connected: bool,
    /// A status has arrived at least once.
    pub seen: bool,
    /// Artifact text by path, as read through the port.
    pub artifacts: HashMap<PathBuf, String>,
    /// The `dispatch` executable the console types.
    pub command: PathBuf,
    /// Dispatch's data directory: the console's working directory.
    pub data_dir: PathBuf,
}

impl AppCore {
    #[must_use]
    pub fn dispatch_state(&self) -> &DispatchState {
        &self.dispatch
    }

    #[must_use]
    pub fn ticket(&self, id: &str) -> Option<&TicketView> {
        self.dispatch.status.tickets.iter().find(|t| t.id == id)
    }

    /// Every pending decision across every ticket, newest ticket first.
    #[must_use]
    pub fn pending_decisions(&self) -> Vec<&DecisionView> {
        self.dispatch
            .status
            .tickets
            .iter()
            .flat_map(|t| t.decisions.iter().filter(|d| d.state == "pending"))
            .collect()
    }

    /// The ticket one of whose attempts ran in `session`.
    #[must_use]
    pub fn ticket_of_session(&self, session: RecordId) -> Option<&TicketView> {
        let id = session.0.to_string();
        self.dispatch
            .status
            .tickets
            .iter()
            .find(|t| t.attempts.iter().any(|a| a.session.as_deref() == Some(&id)))
    }

    /// The console session, if it still exists.
    #[must_use]
    pub fn console(&self) -> Option<RecordId> {
        self.settings
            .dispatch_console
            .filter(|id| self.session(*id).is_some())
    }

    pub(super) fn dispatch_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::DispatchConfigured { command, data_dir } => {
                self.dispatch.command = command;
                self.dispatch.data_dir = data_dir;
            }
            AppAction::DispatchStatus(status) => {
                self.dispatch.connected = status.is_some();
                if let Some(status) = status {
                    self.dispatch.status = status;
                    self.dispatch.seen = true;
                }
            }
            AppAction::ShowDispatch => self.show(View::Dispatch, now, out),
            AppAction::ShowTicket(id) => {
                if self.ticket(&id).is_some() {
                    self.show(View::Ticket(id), now, out);
                } else {
                    self.error(format!("no ticket {id} in the last status"));
                }
            }
            AppAction::DispatchDecide {
                ticket,
                decision,
                answer,
                note,
            } => {
                if !self.dispatch.connected {
                    self.error("Dispatch is not running; start it from the console");
                    return;
                }
                out.push(Effect::DispatchCall(Body::Decide {
                    ticket,
                    decision,
                    answer,
                    note,
                }));
            }
            AppAction::DispatchReadArtifact { ticket, path } => {
                if self.dispatch.artifacts.contains_key(&path) {
                    return;
                }
                out.push(Effect::DispatchCall(Body::Artifact { ticket, path }));
            }
            AppAction::DispatchReplied { body, result } => match (body, result) {
                (_, Err(e)) => {
                    self.dispatch.connected = false;
                    self.error(format!("Dispatch did not answer: {e}"));
                }
                (_, Ok(Reply::Failed { reason })) => self.error(format!("Dispatch: {reason}")),
                (Body::Artifact { path, .. }, Ok(Reply::Artifact { text })) => {
                    self.dispatch.artifacts.insert(path, text);
                }
                (Body::Decide { ticket, .. }, Ok(Reply::Decided(d))) => {
                    // The next status carries it too; this keeps the
                    // buttons from being pressed twice in between.
                    if let Some(t) = self
                        .dispatch
                        .status
                        .tickets
                        .iter_mut()
                        .find(|t| t.id == ticket)
                        && let Some(slot) = t.decisions.iter_mut().find(|x| x.id == d.id)
                    {
                        *slot = d;
                    }
                }
                _ => {}
            },
            AppAction::OpenDispatchConsole => {
                self.ensure_console(now, out);
            }
            AppAction::DispatchConsole(line) => {
                let line = line.trim().to_owned();
                if line.is_empty() {
                    return;
                }
                let Some(id) = self.ensure_console(now, out) else {
                    return;
                };
                let command = self.dispatch.command.display().to_string();
                let text = match line.strip_prefix('!') {
                    Some(shell) => shell.trim().to_owned(),
                    None => format!("'{}' {line}", command.replace('\'', "'\\''")),
                };
                self.session_action(AppAction::SendInput { id, text }, now, out);
            }
            _ => {}
        }
    }

    /// The console session: found, or made in a `Dispatch` space and
    /// project of its own with Dispatch's data directory as its cwd.
    /// The one made is remembered in the settings.
    fn ensure_console(&mut self, now: Clock, out: &mut Out) -> Option<RecordId> {
        if let Some(id) = self.console() {
            return Some(id);
        }
        let found = self
            .views
            .spaces
            .iter()
            .find(|s| s.name == CONSOLE_SPACE)
            .map(|s| s.id);
        let space = found.unwrap_or_else(SpaceId::new);
        if found.is_none() {
            self.update_views(out, |v| {
                v.spaces.push(Space {
                    id: space,
                    name: CONSOLE_SPACE.into(),
                    op: None,
                });
            });
        }
        let root = self.dispatch.data_dir.clone();
        let project = match self
            .workspaces
            .iter()
            .find(|w| w.project.space == space && w.project.name == CONSOLE_SPACE)
            .map(|w| w.project.id)
        {
            Some(id) => id,
            None => self.add_project_record(CONSOLE_SPACE.into(), root.clone(), space, now, out),
        };
        let id = self.add_record(
            project,
            CONSOLE_NAME.into(),
            SessionKind::Shell,
            root,
            Launch::Shell,
            now,
            out,
        )?;
        self.update_settings(out, |s| s.dispatch_console = Some(id));
        self.launch_fresh(id, now, out);
        Some(id)
    }
}
