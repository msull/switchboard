//! Entries from a project's `.switchboard/project.json` become records
//! with a `source`, and run only once the user approved that exact
//! definition. Approval is a hash of the entry stored on the record, so
//! an edit to the file drops it by construction and reverting the edit
//! restores it. Removing an entry orphans its record rather than
//! deleting it: the record still owns scrollback and an exit code.

use std::fmt::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::core::action::{AppCore, Clock, ConfigStatus, Effect, Out};
use crate::core::model::{
    Activity, CardLayout, Definition, Launch, ProjectId, RecordId, SessionKind, SessionRecord,
    Workspace,
};
use crate::ports::project_config::{DefinedEntry, ProjectConfig};

/// Content hash of a definition: everything that changes what runs.
/// The shell is not included; it is the user's, not the file's.
#[must_use]
pub fn entry_hash(entry: &DefinedEntry) -> String {
    let mut h = Sha256::new();
    let kind = match entry.kind {
        SessionKind::Command => "command",
        SessionKind::Service => "service",
        SessionKind::Agent(_) | SessionKind::Shell => "other",
    };
    // A field may contain a newline, so NUL separates them.
    for part in [kind, &entry.name, &entry.command]
        .into_iter()
        .chain(std::iter::once(entry.cwd.as_deref().unwrap_or("")))
        .chain(entry.env.iter().map(String::as_str))
    {
        h.update(part.as_bytes());
        h.update([0]);
    }
    h.update([u8::from(entry.autostart)]);
    h.finalize().iter().fold(String::new(), |mut hex, b| {
        let _ = write!(hex, "{b:02x}");
        hex
    })
}

impl AppCore {
    pub(super) fn project_config_read(
        &mut self,
        project: ProjectId,
        result: Result<Option<ProjectConfig>, String>,
        now: Clock,
        out: &mut Out,
    ) {
        let Some(root) = self.workspace(project).map(|w| w.project.root.clone()) else {
            return;
        };
        let previous = self.config_status(project).cloned().unwrap_or_default();
        let status = match &result {
            Ok(Some(cfg)) => ConfigStatus {
                present: true,
                warnings: cfg.warnings.clone(),
                error: None,
            },
            Ok(None) => ConfigStatus::default(),
            Err(e) => ConfigStatus {
                present: true,
                warnings: Vec::new(),
                error: Some(e.clone()),
            },
        };
        // A poll repeats the same failure every few seconds; one notice
        // per distinct message is enough.
        if let Some(e) = &status.error
            && previous.error.as_deref() != Some(e)
        {
            self.error(format!("{}: {e}", self.project_name(project)));
        }
        self.set_config_status(project, status);

        // The folders to show despite .gitignore follow the file; an
        // absent file shows none.
        let shown = match &result {
            Ok(Some(cfg)) => cfg.show.clone(),
            Ok(None) | Err(_) => Vec::new(),
        };
        if result.is_ok()
            && let Some(workspace) = self.workspaces.iter_mut().find(|w| w.project.id == project)
            && workspace.project.shown != shown
        {
            workspace.project.shown = shown;
            out.touch(project);
        }

        let entries = match result {
            Ok(Some(cfg)) => cfg
                .entries
                .into_iter()
                .map(|e| (e, cfg.shell.clone()))
                .collect(),
            Ok(None) => Vec::new(),
            // Keep the records as they were: an unreadable file says
            // nothing about what it used to declare.
            Err(_) => return,
        };
        let Some(workspace) = self.workspaces.iter_mut().find(|w| w.project.id == project) else {
            return;
        };
        let before = workspace.sessions.clone();
        let mut seen: Vec<RecordId> = Vec::new();
        for (entry, shell) in entries {
            seen.push(upsert(workspace, &root, entry, shell, now));
        }
        for record in &mut workspace.sessions {
            if let Some(d) = &mut record.source
                && !seen.contains(&record.id)
            {
                d.orphaned = true;
            }
        }
        if workspace.sessions != before {
            out.touch(project);
        }
    }

    pub(super) fn approve_definition(&mut self, id: RecordId, out: &mut Out) {
        let hash = self
            .session(id)
            .and_then(|s| s.source.as_ref())
            .filter(|d| !d.orphaned)
            .map(|d| d.hash.clone());
        if let Some(hash) = hash {
            self.edit_session(id, out, |s| s.approved_hash = Some(hash));
        } else {
            let name = self.session_name(id);
            self.error(format!("{name} has no definition to approve"));
        }
    }

    pub(super) fn revoke_approval(&mut self, id: RecordId, out: &mut Out) {
        self.edit_session(id, out, |s| s.approved_hash = None);
    }

    fn set_config_status(&mut self, project: ProjectId, status: ConfigStatus) {
        match self.config_status.iter_mut().find(|(p, _)| *p == project) {
            Some((_, s)) => *s = status,
            None => self.config_status.push((project, status)),
        }
    }

    fn project_name(&self, project: ProjectId) -> String {
        self.workspace(project)
            .map_or_else(|| "project".into(), |w| w.project.name.clone())
    }
}

/// Bring the record for one entry up to date, or create it. The file
/// owns kind, cwd, launch, and the definition; approval, history, and
/// layout stay with the record. Returns the record's id.
fn upsert(
    workspace: &mut Workspace,
    root: &Path,
    entry: DefinedEntry,
    shell: String,
    now: Clock,
) -> RecordId {
    let cwd = entry
        .cwd
        .as_deref()
        .map_or_else(|| root.to_path_buf(), |c| root.join(c));
    let launch = Launch::Command {
        command: entry.command.clone(),
        shell,
    };
    let definition = Definition {
        name: entry.name.clone(),
        hash: entry_hash(&entry),
        env: entry.env.clone(),
        autostart: entry.autostart,
        orphaned: false,
    };
    let existing = workspace
        .sessions
        .iter_mut()
        .find(|s| s.source.as_ref().is_some_and(|d| d.name == entry.name));
    if let Some(record) = existing {
        record.kind = entry.kind;
        record.cwd = cwd;
        record.launch = launch;
        record.source = Some(definition);
        return record.id;
    }
    let order = workspace
        .sessions
        .iter()
        .map(|s| s.layout.order + 1)
        .max()
        .unwrap_or(0);
    let id = RecordId::new();
    workspace.sessions.push(SessionRecord {
        id,
        project: workspace.project.id,
        name: entry.name,
        kind: entry.kind,
        cwd,
        launch,
        env_profile: None,
        created: now.wall,
        last_seen: now.wall,
        notes: String::new(),
        resume: None,
        autostart: false,
        layout: CardLayout { order, group: None },
        activity: Activity::Unknown,
        activity_reason: None,
        last_event_at: None,
        last_exit: None,
        not_resumable: false,
        scrollback: None,
        source: Some(definition),
        approved_hash: None,
        discard: None,
    });
    id
}

/// The effect that asks the app to read a project's definition file.
#[must_use]
pub(super) fn read_config(project: ProjectId, root: PathBuf) -> Effect {
    Effect::ReadProjectConfig { project, root }
}
