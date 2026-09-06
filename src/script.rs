//! Dev aid: `SWITCHBOARD_SCRIPT=<file>` dispatches one action per line
//! after startup, so the real app can be put into a known state without
//! clicking. Lines: `add-project <name> <root>`, `new-shell <project>
//! <name>`, `new-claude <project> <name>`, `new-codex <project> <name>`,
//! `new-service <project> <name> <command...>`, `show-board <project>`,
//! `show-session <name>`, `return <name>`, `kill <name>`, `switchboard`.
//! Blank lines and `#` comments are ignored; unknown lines are logged.

use std::path::PathBuf;

use crate::app::SwitchboardApp;
use crate::core::{AgentKind, AppAction, Launch, ProjectId, RecordId, SessionKind};

pub fn run(app: &mut SwitchboardApp, text: &str) {
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        if let Err(e) = step(app, &words) {
            log::warn!("script: {line}: {e}");
        }
    }
}

fn project(app: &SwitchboardApp, name: &str) -> Result<(ProjectId, PathBuf), String> {
    app.core()
        .workspaces()
        .iter()
        .find(|w| w.project.name == name)
        .map(|w| (w.project.id, w.project.root.clone()))
        .ok_or_else(|| format!("no project {name}"))
}

fn session(app: &SwitchboardApp, name: &str) -> Result<RecordId, String> {
    app.core()
        .workspaces()
        .iter()
        .flat_map(|w| &w.sessions)
        .find(|s| s.name == name)
        .map(|s| s.id)
        .ok_or_else(|| format!("no session {name}"))
}

fn new_session(
    app: &mut SwitchboardApp,
    proj: &str,
    name: &str,
    kind: SessionKind,
    launch: Launch,
) -> Result<(), String> {
    let (project, cwd) = project(app, proj)?;
    app.dispatch(AppAction::NewSession {
        project,
        name: name.to_owned(),
        kind,
        cwd,
        launch,
    });
    Ok(())
}

fn step(app: &mut SwitchboardApp, w: &[&str]) -> Result<(), String> {
    match w {
        ["add-project", name, root] => app.dispatch(AppAction::AddProject {
            name: (*name).to_owned(),
            root: PathBuf::from(root),
        }),
        ["new-shell", p, n] => new_session(app, p, n, SessionKind::Shell, Launch::Shell)?,
        ["new-claude", p, n] => new_session(
            app,
            p,
            n,
            SessionKind::Agent(AgentKind::ClaudeCode),
            Launch::Shell,
        )?,
        ["new-codex", p, n] => {
            new_session(
                app,
                p,
                n,
                SessionKind::Agent(AgentKind::Codex),
                Launch::Shell,
            )?;
        }
        ["new-service", p, n, cmd @ ..] => new_session(
            app,
            p,
            n,
            SessionKind::Service,
            Launch::Command {
                command: cmd.join(" "),
                shell: "/bin/zsh".into(),
            },
        )?,
        ["show-board", p] => {
            let (id, _) = project(app, p)?;
            app.dispatch(AppAction::ShowBoard(id));
        }
        ["show-session", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::ShowSession(id));
        }
        ["return", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::ReturnToSession(id));
        }
        ["kill", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::KillSession(id));
        }
        ["switchboard"] => app.dispatch(AppAction::ShowSwitchboard),
        _ => return Err("unknown line".into()),
    }
    Ok(())
}
