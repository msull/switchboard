//! Standing agents: agent records whose identity is a directory under
//! `<data dir>/standing/`, which holds their instructions and the notes,
//! ledger and hand-off they keep. Restart is a kill and a fresh launch
//! in the same directory, never a resume, so the files are the memory.
//!
//! The launch prompt is owed to every new session (`brief_pending`).
//! Claude Code gets it typed into the pane on the first of two releases:
//! the new session's `SessionStart` (when no trust question is showing)
//! or the owner's answer to the trust question. Codex has no hooks, and
//! its rollout id exists only after a first prompt, so its prompt rides
//! the command line.

use std::path::Path;
use std::time::SystemTime;

use super::action::{AppAction, AppCore, Clock, Effect, Out};
use super::model::{AgentKind, Launch, RecordId, SessionKind, SessionRecord, Standing};
use crate::ports::events::{EventKind, SessionEvent};
use crate::ports::standing::PreparedStanding;

/// The data directory's folder that holds the standing agents.
pub const STANDING_DIR: &str = "standing";

/// The name of the project that holds the standing agents.
pub const STANDING_PROJECT: &str = "Standing agents";

/// The longest a standing agent's name may be.
const NAME_MAX: usize = 64;

/// The line between Switchboard's base text and the owner's.
pub const INSTRUCTIONS_MARKER: &str =
    "<!-- The owner's instructions follow. Switchboard rewrites everything above this line. -->";

/// How a standing agent keeps its memory, said once so the instructions
/// file and the launch prompt cannot disagree. One line, because the
/// prompt is typed into the pane and a newline would submit it early.
const CONVENTION: &str = "This directory is your memory, and nothing else carries over between sessions: append a dated line to ledger.md as each piece of work goes, keep notes.md current with what you know and are working on, and rewrite handoff.md when a piece of work ends, saying where things stand and what comes next.";

/// What Switchboard writes above the marker in the instructions file.
#[must_use]
pub fn base_instructions(agent: AgentKind) -> String {
    format!(
        "# Standing agent\n\nYou are a standing agent run by Switchboard ({}). You are restarted into fresh sessions in this directory and never resumed.\n\n{CONVENTION}\n\nAt the start of a session read notes.md, handoff.md and the last 40 lines of ledger.md. If any is missing, say which and wait for the owner.\n",
        agent.label()
    )
}

/// The first prompt of every standing session.
#[must_use]
pub fn launch_prompt(agent: AgentKind) -> String {
    format!(
        "You are a standing agent and this directory is yours. Read {} here if it is not already in your context, then notes.md, handoff.md and the last 40 lines of ledger.md in this directory. If any of them is missing, say which and wait for the owner. {CONVENTION}",
        agent.instructions_file()
    )
}

/// Whether `name` can be a standing agent's directory: one path
/// component of letters, digits, `-`, `_` and `.`, not starting with a
/// dot, so it can neither escape the standing root nor hide.
#[must_use]
pub fn valid_standing_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

impl AppCore {
    /// The standing agents' transitions, split out of `dispatch`.
    pub(super) fn standing_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::NewStanding {
                name,
                agent,
                instructions,
                launch_flags,
            } => self.new_standing(&name, agent, instructions, launch_flags, out),
            AppAction::StandingPrepared {
                name,
                agent,
                launch_flags,
                result,
            } => self.standing_prepared(&name, agent, launch_flags, result, now, out),
            AppAction::StandingSeen {
                id,
                hash,
                handoff_at,
            } => self.standing_seen(id, hash, handoff_at, out),
            AppAction::SaveStandingInstructions { id, text } => {
                let Some((agent, dir)) = self
                    .standing_record(id)
                    .map(|(agent, dir)| (agent, dir.to_path_buf()))
                else {
                    self.error("not a standing agent");
                    return;
                };
                out.push(Effect::WriteStandingInstructions {
                    id,
                    agent,
                    dir,
                    text,
                });
            }
            AppAction::StandingInstructionsWritten { id, result } => {
                let name = self.session_name(id);
                match result {
                    Ok(hash) => {
                        self.set_standing_seen(id, Some(hash));
                        self.info_about(
                            id,
                            format!("{name}: instructions saved; Restart to use them"),
                            now,
                        );
                    }
                    Err(e) => self.error_about(id, format!("{name}: instructions not saved: {e}")),
                }
            }
            AppAction::RemoveStanding(id) => self.remove_standing(id, now, out),
            _ => unreachable!("not a standing action"),
        }
    }

    /// A standing record's agent kind and directory.
    fn standing_record(&self, id: RecordId) -> Option<(AgentKind, &Path)> {
        let record = self.session(id)?;
        record.standing.as_ref()?;
        match record.kind {
            SessionKind::Agent(agent) => Some((agent, record.cwd.as_path())),
            _ => None,
        }
    }

    /// The standing agent whose directory is called `name`, if any,
    /// removed ones still in their undo window included: the directory
    /// is the identity, and a macOS volume does not tell `Ops` from
    /// `ops`, so the names are compared without case.
    fn standing_named(&self, name: &str) -> Option<RecordId> {
        let named = |s: &&SessionRecord| {
            s.standing.is_some()
                && s.cwd
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
        };
        self.workspaces
            .iter()
            .flat_map(|w| &w.sessions)
            .chain(self.trashed_sessions())
            .find(named)
            .map(|s| s.id)
    }

    fn new_standing(
        &mut self,
        name: &str,
        agent: AgentKind,
        instructions: Option<String>,
        launch_flags: Vec<String>,
        out: &mut Out,
    ) {
        let name = name.trim().to_owned();
        if !valid_standing_name(&name) {
            self.error(format!(
                "{name:?} cannot name a standing agent: use letters, digits, '-', '_' and '.'"
            ));
            return;
        }
        if self.standing_named(&name).is_some() {
            self.error(format!("{name} is already a standing agent"));
            return;
        }
        if self
            .standing_ops
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case(&name))
        {
            self.error(format!("{name} is already being made"));
            return;
        }
        if let Some(text) = self.host_unavailable("start", &name) {
            self.error(text);
            return;
        }
        // The record is made when the directory is ready, one action
        // later; the control port's operation has to follow it there.
        if let Some(op) = self.quiet_op.clone() {
            self.standing_ops.push((name.clone(), op));
        }
        out.push(Effect::PrepareStanding {
            name,
            agent,
            instructions,
            launch_flags,
        });
    }

    fn standing_prepared(
        &mut self,
        name: &str,
        agent: AgentKind,
        launch_flags: Vec<String>,
        result: Result<PreparedStanding, String>,
        now: Clock,
        out: &mut Out,
    ) {
        let op = self
            .standing_ops
            .iter()
            .position(|(n, _)| n == name)
            .map(|i| self.standing_ops.remove(i).1);
        // The record is made under the command's operation, so the
        // control port owns it.
        let outer = std::mem::replace(&mut self.quiet_op, op.clone());
        let made = self.make_standing(name, agent, launch_flags, result, now, out);
        self.quiet_op = outer;
        let Some(op) = op else {
            if let Err(e) = made {
                self.error(e);
            }
            return;
        };
        // The command's outcome was written when it was asked for, with
        // nothing made; it is completed here, before the app reads it.
        let Some(outcome) = self.control_outcomes.iter_mut().find(|o| o.op == op) else {
            return;
        };
        match made {
            Ok(id) => {
                outcome.made.push(switchboard_control::Made {
                    kind: switchboard_control::RecordKind::Session,
                    id: id.0.to_string(),
                });
                out.push(Effect::LogOperation {
                    op,
                    kind: "standing.new".to_owned(),
                    ids: vec![id.0.to_string()],
                });
            }
            Err(error) => {
                outcome.error = Some(match outcome.error.take() {
                    Some(before) => format!("{before}; {error}"),
                    None => error,
                });
            }
        }
    }

    /// The record for a prepared directory, in the standing project
    /// (made on first use), launched with its brief.
    fn make_standing(
        &mut self,
        name: &str,
        agent: AgentKind,
        launch_flags: Vec<String>,
        result: Result<PreparedStanding, String>,
        now: Clock,
        out: &mut Out,
    ) -> Result<RecordId, String> {
        let prepared =
            result.map_err(|e| format!("could not prepare standing agent {name}: {e}"))?;
        if self.standing_named(name).is_some() {
            return Err(format!("{name} is already a standing agent"));
        }
        let existing = self
            .workspaces
            .iter()
            .find(|w| w.project.root == prepared.root)
            .map(|w| w.project.id);
        let project = if let Some(project) = existing {
            project
        } else {
            let space = self.add_project_space();
            self.add_project_record(
                STANDING_PROJECT.to_owned(),
                prepared.root.clone(),
                space,
                now,
                out,
            )
        };
        let id = self
            .add_record(
                project,
                name.to_owned(),
                SessionKind::Agent(agent),
                prepared.dir,
                Launch::Argv(launch_flags),
                now,
                out,
            )
            .ok_or_else(|| format!("could not make standing agent {name}"))?;
        self.edit_session(id, out, |s| s.standing = Some(Standing::default()));
        self.set_standing_seen(id, Some(prepared.hash));
        self.launch_standing(id, false, now, out);
        Ok(id)
    }

    /// A standing record's launch, first or restarted: the seen
    /// instructions become the launched ones, and the brief is owed.
    pub(super) fn launch_standing(
        &mut self,
        id: RecordId,
        restart: bool,
        now: Clock,
        out: &mut Out,
    ) {
        let Some((agent, _)) = self.standing_record(id) else {
            return;
        };
        let seen = self.standing_seen_hash(id).map(str::to_owned);
        self.edit_session(id, out, |s| {
            if let Some(standing) = &mut s.standing {
                standing.seed_hash = seen;
                if restart {
                    standing.restart_count += 1;
                }
            }
        });
        // A pane killed before its release leaves this set, which is
        // harmless: only the next launch's session can take it, and a
        // return by resume clears it.
        self.set_brief_pending(id, true, out);
        self.brief_sent.retain(|r| *r != id);
        self.first_prompts.retain(|(r, _)| *r != id);
        if agent == AgentKind::Codex {
            self.first_prompts.push((id, launch_prompt(agent)));
        }
        self.launch_fresh(id, now, out);
    }

    /// Type the owed launch prompt into the pane, once. Sent at a
    /// `SessionStart` the flag clears at once; sent with the answer to
    /// the trust question (`confirm`), the dialog may have eaten it, so
    /// the flag stays until the session shows a submitted prompt and the
    /// "brief not sent" chip can still say it never went in.
    pub(super) fn release_brief(&mut self, id: RecordId, confirm: bool, out: &mut Out) {
        let Some((agent, _)) = self.standing_record(id) else {
            return;
        };
        let owed = self
            .session(id)
            .and_then(|s| s.standing.as_ref())
            .is_some_and(|s| s.brief_pending);
        if !owed || self.brief_sent.contains(&id) {
            return;
        }
        let Some(host) = self.running_host(id) else {
            return;
        };
        if confirm {
            self.brief_sent.push(id);
        } else {
            self.set_brief_pending(id, false, out);
        }
        out.push(Effect::SendInput {
            host,
            text: launch_prompt(agent),
        });
    }

    /// A prompt was submitted in a session whose launch prompt went with
    /// the trust answer: it arrived, so it is no longer owed.
    pub(super) fn brief_confirmed(&mut self, id: RecordId, out: &mut Out) {
        if self.brief_sent.contains(&id) {
            self.brief_sent.retain(|r| *r != id);
            self.set_brief_pending(id, false, out);
        }
    }

    /// Set or clear the owed launch prompt on a standing record.
    pub(super) fn set_brief_pending(&mut self, id: RecordId, owed: bool, out: &mut Out) {
        self.edit_session(id, out, |s| {
            if let Some(standing) = &mut s.standing {
                standing.brief_pending = owed;
            }
        });
    }

    /// Whether `event` releases a Claude Code standing agent's launch
    /// prompt: it is the new session's own start (its id was set before
    /// the spawn), so the old pane's late events, a later `/clear` or
    /// `/resume`, and an event with no id never release it; and no trust
    /// question shows, whose answer releases it instead.
    pub(super) fn brief_due(&self, id: RecordId, event: &SessionEvent) -> bool {
        event.kind == EventKind::SessionStart
            && self.claude_brief_pending(id)
            && !self.brief_sent.contains(&id)
            && !self.prompted.contains(&id)
            && event.provider_session_id.as_deref().is_some_and(|p| {
                self.session(id)
                    .and_then(|s| s.resume.as_ref())
                    .is_some_and(|h| h.provider_id() == p)
            })
    }

    /// The brief is owed and this record is a Claude Code standing agent.
    pub(super) fn claude_brief_pending(&self, id: RecordId) -> bool {
        self.session(id).is_some_and(|s| {
            s.kind == SessionKind::Agent(AgentKind::ClaudeCode)
                && s.standing.as_ref().is_some_and(|st| st.brief_pending)
        })
    }

    /// The brief is owed and this record is a Codex standing agent.
    fn codex_brief_pending(&self, id: RecordId) -> bool {
        self.session(id).is_some_and(|s| {
            s.kind == SessionKind::Agent(AgentKind::Codex)
                && s.standing.as_ref().is_some_and(|st| st.brief_pending)
        })
    }

    /// A Codex standing agent's rollout bound, so its prompt reached it.
    pub(super) fn codex_brief_delivered(&mut self, id: RecordId, out: &mut Out) {
        if self.codex_brief_pending(id) {
            self.set_brief_pending(id, false, out);
        }
    }

    fn standing_seen(
        &mut self,
        id: RecordId,
        hash: Option<String>,
        handoff_at: Option<SystemTime>,
        out: &mut Out,
    ) {
        if self.standing_record(id).is_none() {
            return;
        }
        // A launch before the first poll had no hash to record; the
        // file as first seen is the one it was launched from.
        let unseeded = self
            .session(id)
            .and_then(|s| s.standing.as_ref())
            .is_some_and(|s| s.seed_hash.is_none());
        if unseeded && hash.is_some() {
            let seed = hash.clone();
            self.edit_session(id, out, |s| {
                if let Some(standing) = &mut s.standing {
                    standing.seed_hash = seed;
                }
            });
        }
        self.set_standing_seen(id, hash);
        let moved = self
            .session(id)
            .and_then(|s| s.standing.as_ref())
            .is_some_and(|s| s.handoff_at != handoff_at);
        if moved {
            self.edit_session(id, out, |s| {
                if let Some(standing) = &mut s.standing {
                    standing.handoff_at = handoff_at;
                }
            });
        }
    }

    fn set_standing_seen(&mut self, id: RecordId, hash: Option<String>) {
        self.standing_hashes.retain(|(r, _)| *r != id);
        if let Some(hash) = hash {
            self.standing_hashes.push((id, hash));
        }
    }

    fn standing_seen_hash(&self, id: RecordId) -> Option<&str> {
        self.standing_hashes
            .iter()
            .find(|(r, _)| *r == id)
            .map(|(_, h)| h.as_str())
    }

    /// The record goes and the directory stays, so a standing agent made
    /// again under the same name finds its memory. A record already gone
    /// is removed, as far as the caller can tell.
    fn remove_standing(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        if self.session(id).is_none() {
            return;
        }
        if self.standing_record(id).is_none() {
            let name = self.session_name(id);
            self.error(format!("{name} is not a standing agent"));
            return;
        }
        self.kill_and_forget(id, out);
        self.standing_hashes.retain(|(r, _)| *r != id);
        self.first_prompts.retain(|(r, _)| *r != id);
        self.trash_session(id, now);
    }

    /// The control port's restart: only a standing agent's.
    pub(super) fn restart_standing(&mut self, id: RecordId, now: Clock, out: &mut Out) {
        if self.standing_record(id).is_none() {
            let name = self.session_name(id);
            self.error(format!("{name} is not a standing agent"));
            return;
        }
        self.restart_session(id, now, out);
    }

    /// Whether the instructions file changed since the agent's last
    /// launch, so a Restart is due. One whose file was not seen yet is
    /// not stale.
    #[must_use]
    pub fn standing_stale(&self, id: RecordId) -> bool {
        let Some(standing) = self.session(id).and_then(|s| s.standing.as_ref()) else {
            return false;
        };
        self.standing_seen_hash(id)
            .is_some_and(|seen| standing.seed_hash.as_deref() != Some(seen))
    }

    /// Whether the running session should have had its launch prompt
    /// by now and has not: for Claude Code, a hook has reported since
    /// the spawn and no trust question is showing; for Codex, discovery
    /// ended with no rollout bound.
    #[must_use]
    pub fn standing_brief_unsent(&self, id: RecordId) -> bool {
        let Some(record) = self.session(id) else {
            return false;
        };
        if !record.standing.as_ref().is_some_and(|s| s.brief_pending) || !self.is_running(id) {
            return false;
        }
        match record.kind {
            SessionKind::Agent(AgentKind::ClaudeCode) => {
                let spawned = self
                    .spawned_at
                    .iter()
                    .find(|(r, _)| *r == id)
                    .map(|(_, at)| *at);
                !self.prompted.contains(&id)
                    && record
                        .last_event_at
                        .is_some_and(|at| spawned.is_none_or(|s| at >= s))
            }
            SessionKind::Agent(AgentKind::Codex) => {
                !self.is_in_flight(id) && record.resume.is_none()
            }
            _ => false,
        }
    }
}
