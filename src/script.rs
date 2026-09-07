//! Dev aid: `SWITCHBOARD_SCRIPT=<file>` dispatches one action per line
//! after startup, so the real app can be put into a known state without
//! clicking. Lines: `add-project <name> <root>`, `new-shell <project>
//! <name>`, `new-claude <project> <name>`, `new-codex <project> <name>`,
//! `new-service <project> <name> <command...>`, `show-board <project>`,
//! `show-session <name>`, `show-document <project> <relative path>`,
//! `set-env <project> NAME=VALUE`, `set-secret <project> NAME VALUE`,
//! `dotenv <project> on|off`, `environment <project>` (opens the dialog), `send <name> <text...>`, `return <name>`,
//! `kill <name>`, `switchboard`, `sleep <secs>` (then polls).
//! Blank lines and `#` comments are ignored; unknown lines are logged.

use std::path::PathBuf;

use crate::app::SwitchboardApp;
use crate::core::{
    AgentKind, AppAction, EnvVar, Launch, ProjectId, RecordId, SecretScope, SessionKind,
};

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
        ["show-document", p, rel] => {
            let (id, root) = project(app, p)?;
            app.dispatch(AppAction::ShowDocument(id, root.join(rel)));
        }
        ["set-env" | "set-secret" | "dotenv" | "environment", ..] => env_step(app, w)?,
        ["show-session", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::ShowSession(id));
        }
        ["return", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::ReturnToSession(id));
        }
        ["send", n, text @ ..] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::SendInput {
                id,
                text: text.join(" "),
            });
        }
        ["kill", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::KillSession(id));
        }
        ["switchboard"] => app.dispatch(AppAction::ShowSwitchboard),
        ["sleep", secs] => {
            let secs: u64 = secs.parse().map_err(|_| "bad sleep")?;
            std::thread::sleep(std::time::Duration::from_secs(secs));
            app.poll_now();
        }
        _ => return Err("unknown line".into()),
    }
    Ok(())
}

/// The environment lines, split out of `step` for length.
fn env_step(app: &mut SwitchboardApp, w: &[&str]) -> Result<(), String> {
    match w {
        ["set-env", p, pair] => {
            let (id, _) = project(app, p)?;
            let (name, value) = pair
                .split_once('=')
                .ok_or_else(|| format!("set-env wants NAME=VALUE, got {pair}"))?;
            let mut env = app
                .core()
                .workspace(id)
                .map(|w| w.project.env.clone())
                .unwrap_or_default();
            env.vars.retain(|v| v.name != name);
            env.vars.push(EnvVar {
                name: name.into(),
                value: value.into(),
                secret: false,
            });
            app.dispatch(AppAction::SetProjectEnv(id, env));
        }
        ["set-secret", p, name, value] => {
            let (id, _) = project(app, p)?;
            let mut env = app
                .core()
                .workspace(id)
                .map(|w| w.project.env.clone())
                .unwrap_or_default();
            env.vars.retain(|v| v.name != *name);
            env.vars.push(EnvVar {
                name: (*name).into(),
                value: String::new(),
                secret: true,
            });
            app.dispatch(AppAction::SetProjectEnv(id, env));
            app.dispatch(AppAction::StoreSecret {
                scope: SecretScope::Project(id),
                name: (*name).into(),
                value: (*value).into(),
            });
        }
        ["dotenv", p, on] => {
            let (id, _) = project(app, p)?;
            let mut env = app
                .core()
                .workspace(id)
                .map(|w| w.project.env.clone())
                .unwrap_or_default();
            env.load_dotenv = *on == "on";
            app.dispatch(AppAction::SetProjectEnv(id, env));
        }
        ["environment", p] => {
            let (id, _) = project(app, p)?;
            app.ui_state.env_dialog =
                crate::ui::env::EnvDraft::project(app.core(), app.services(), id);
        }
        _ => return Err(format!("unknown environment line: {}", w.join(" "))),
    }
    Ok(())
}
