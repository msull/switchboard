//! Dev aid: `SWITCHBOARD_SCRIPT=<file>` dispatches one action per line
//! after startup, so the real app can be put into a known state without
//! clicking. Lines: `add-project <name> <root>`, `new-shell <project>
//! <name>`, `new-claude <project> <name>`, `new-codex <project> <name>`,
//! `new-service <project> <name> <command...>`, `show-board <project>`,
//! `show-session <name>`, `show-document <project> <relative path>`,
//! `files on|off` (the file side of a session), `side-position left|right`
//! (where the side panel sits), `terminal on|off` (the
//! raw pane under a conversation), `select-file <project>
//! <relative path>` (previewed in that side),
//! `set-env <project> NAME=VALUE`, `set-secret <project> NAME VALUE`,
//! `dotenv <project> on|off`, `environment <project>` (opens the dialog),
//! `config <project>` (opens the config editor),
//! `review-plan <session> <absolute plan path>` (starts a plan review),
//! `show-review` (the newest review's page), `review-file
//! feedback|response <first line...>` (writes the newest review's
//! awaited file to disk, as its agent would), `review-continue`,
//! `review-finalize`, `show-artifact <name> <index>` (a command's card
//! and page show that file of its last run), `pop-out <name>` and
//! `close-pop-out <name>` (a session's own window), `files-root
//! <project> <relative dir|.>` (where the file side's tree starts),
//! `send <name> <text...>`, `interrupt <name>` (Escape to the pane),
//! `return <name>`, `kill <name>`, `approve <name>`, `revoke <name>`
//! (a defined command's approval), `side files|run|notes` (the side panel's
//! tab), `switchboard`, `theme light|dark|auto`, `sleep <secs>` (then
//! polls).
//! Blank lines and `#` comments are ignored; unknown lines are logged.

use std::path::PathBuf;

use crate::app::SwitchboardApp;
use crate::core::{
    AgentKind, AppAction, EnvVar, Launch, PinTarget, ProjectId, RecordId, SecretScope, SessionKind,
    SetId, SideTab,
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
        outputs: Vec::new(),
    });
    Ok(())
}

/// The working-set lines (show it, put a session on it by name, put a
/// file on it by project and relative path), the message dialog, the
/// theme, and sleep: what did not fit in `step`.
/// The lines `working_set_step` handles, kept out of `step` for length.
const EXTRA_LINES: &[&str] = &[
    "switchboard",
    "working-set",
    "new-working-set",
    "clone-working-set",
    "rename-working-set",
    "delete-working-set",
    "add-to-working-set",
    "add-file-to-working-set",
    "arrange",
    "show-message",
    "clone-session",
    "discard-to",
    "undo-discard",
    "open-terminal",
    "prompt-box",
    "theme",
    "sleep",
];

/// The lines `review_step` handles.
const REVIEW_LINES: &[&str] = &[
    "review-plan",
    "show-review",
    "review-file",
    "review-continue",
    "review-finalize",
    "show-artifact",
    "pop-out",
    "close-pop-out",
    "files-root",
    "zoom",
    "place-pop-out",
];

/// The plan review lines, kept out of `working_set_step` for length.
fn review_step(app: &mut SwitchboardApp, w: &[&str]) -> Result<(), String> {
    match w {
        ["pop-out", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::PopOut(id));
        }
        ["close-pop-out", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::ClosePopout(id));
        }
        ["files-root", p, rel] => {
            let (id, _) = project(app, p)?;
            let dir = (*rel != ".").then(|| PathBuf::from(rel));
            app.dispatch(AppAction::SetFileRoot(id, dir));
        }
        ["zoom", percent, monitor @ ..] => {
            let percent = percent.parse().map_err(|_| "zoom: percent".to_owned())?;
            app.dispatch(AppAction::SetMonitorZoom(monitor.join(" "), percent));
        }
        ["place-pop-out", name, left, top, width, height] => {
            let id = session(app, name)?;
            let num = |v: &str| v.parse::<i32>().map_err(|_| format!("place-pop-out: {v}"));
            let frame = crate::core::WindowFrame {
                x: num(left)?,
                y: num(top)?,
                w: num(width)?,
                h: num(height)?,
            };
            app.dispatch(AppAction::PopoutMoved(id, frame));
        }
        ["review-plan", name, path] => {
            let source = session(app, name)?;
            app.dispatch(AppAction::StartWorkflow {
                source,
                plan: PathBuf::from(path),
                definition: crate::core::BUILTIN_WORKFLOW.into(),
            });
        }
        ["show-review"] => {
            let id = newest_review(app)?;
            app.dispatch(AppAction::ShowWorkflow(id));
        }
        ["review-continue"] => {
            let id = newest_review(app)?;
            app.dispatch(AppAction::ContinueWorkflow(id));
        }
        ["review-finalize"] => {
            let id = newest_review(app)?;
            app.dispatch(AppAction::FinalizeWorkflow(id));
        }
        ["review-file", which, text @ ..] => {
            let id = newest_review(app)?;
            let run = app.core().workflow(id).ok_or("no review")?;
            let round = run.current().ok_or("no round")?;
            let path = match *which {
                "feedback" => round.feedback.clone(),
                "response" => round.response.clone(),
                _ => return Err("review-file feedback|response".into()),
            };
            let body = format!("{}\n", text.join(" "));
            std::fs::write(&path, body).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        ["show-artifact", name, index] => {
            let id = session(app, name)?;
            let index: usize = index.parse().map_err(|_| "bad index")?;
            app.ui_state
                .run_modes
                .insert(id, crate::ui::runs::RunCardMode::Artifact(index));
        }
        _ => return Err("unknown review line".into()),
    }
    Ok(())
}

fn newest_review(app: &SwitchboardApp) -> Result<crate::core::WorkflowId, String> {
    app.core()
        .workflows()
        .max_by_key(|r| r.created)
        .map(|r| r.id)
        .ok_or_else(|| "no plan review".to_owned())
}

fn working_set(app: &SwitchboardApp, name: &str) -> Result<SetId, String> {
    app.core()
        .working_sets()
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.id)
        .ok_or_else(|| format!("no working set {name}"))
}

fn add_to_set(
    app: &mut SwitchboardApp,
    name: Option<&str>,
    target: PinTarget,
) -> Result<(), String> {
    let set = match name {
        Some(name) => Some(working_set(app, name)?),
        None => app.core().working_sets().first().map(|s| s.id),
    };
    match set {
        Some(set) => app.dispatch(AppAction::AddToWorkingSet {
            set,
            target,
            columns: 24,
        }),
        None => app.dispatch(AppAction::NewWorkingSet {
            name: None,
            clone_of: None,
            with: Some(target),
            columns: 24,
        }),
    }
    Ok(())
}

/// A session and one of its turns, with that turn's prompt (empty when
/// the conversation is not loaded yet).
fn turn_of(
    app: &SwitchboardApp,
    name: &str,
    turn: &str,
) -> Result<(RecordId, usize, String), String> {
    let id = session(app, name)?;
    let before: usize = turn.parse().map_err(|_| "bad turn number")?;
    let prompt = app
        .ui_state
        .conversations
        .get(&id)
        .and_then(|(_, c)| c.turns.iter().find(|t| t.n == before))
        .map(|t| t.user.clone())
        .unwrap_or_default();
    Ok((id, before, prompt))
}

fn working_set_step(app: &mut SwitchboardApp, w: &[&str]) -> Result<(), String> {
    match w {
        ["switchboard"] => app.dispatch(AppAction::ShowSwitchboard),
        ["open-terminal", on] => {
            app.dispatch(AppAction::SetOpenTerminalOnLaunch(*on == "on"));
        }
        ["prompt-box", on] => app.dispatch(AppAction::SetPromptBox(*on == "on")),
        ["clone-session", name, turn] => {
            let (id, before, prompt) = turn_of(app, name, turn)?;
            app.dispatch(AppAction::CloneSession { id, before, prompt });
        }
        ["discard-to", name, turn] => {
            let (id, before, prompt) = turn_of(app, name, turn)?;
            app.dispatch(AppAction::DiscardTo { id, before, prompt });
        }
        ["undo-discard", name] => {
            let id = session(app, name)?;
            app.dispatch(AppAction::UndoDiscard(id));
        }
        ["sleep", secs] => {
            let secs: u64 = secs.parse().map_err(|_| "bad sleep")?;
            std::thread::sleep(std::time::Duration::from_secs(secs));
            app.poll_now();
        }
        ["theme", mode] => app.dispatch(AppAction::SetTheme(match *mode {
            "dark" => crate::core::ThemeMode::Dark,
            "light" => crate::core::ThemeMode::Light,
            _ => crate::core::ThemeMode::Auto,
        })),
        // `working-set` alone shows the first set (made if none);
        // with a name, that set.
        ["working-set"] => match app.core().working_sets().first() {
            Some(set) => app.dispatch(AppAction::ShowWorkingSet(set.id)),
            None => app.dispatch(AppAction::NewWorkingSet {
                name: None,
                clone_of: None,
                with: None,
                columns: 24,
            }),
        },
        ["working-set", name] => {
            let id = working_set(app, name)?;
            app.dispatch(AppAction::ShowWorkingSet(id));
        }
        ["new-working-set", name] => app.dispatch(AppAction::NewWorkingSet {
            name: Some((*name).to_owned()),
            clone_of: None,
            with: None,
            columns: 24,
        }),
        ["clone-working-set", name] => {
            let id = working_set(app, name)?;
            app.dispatch(AppAction::NewWorkingSet {
                name: None,
                clone_of: Some(id),
                with: None,
                columns: 24,
            });
        }
        ["rename-working-set", name, to] => {
            let set = working_set(app, name)?;
            app.dispatch(AppAction::RenameWorkingSet {
                set,
                name: (*to).to_owned(),
            });
        }
        ["delete-working-set", name] => {
            let id = working_set(app, name)?;
            app.dispatch(AppAction::DeleteWorkingSet(id));
        }
        ["arrange", on] => app.ui_state.arrange.on = *on == "on",
        // The message dialog with `text` (`\n` for a line break), raw
        // or rendered.
        ["show-message", mode, text @ ..] => {
            app.ui_state.raw_message = Some(text.join(" ").replace("\\n", "\n"));
            app.ui_state.message_view = if *mode == "rendered" {
                crate::ui::dialogs::MessageView::Rendered
            } else {
                crate::ui::dialogs::MessageView::Raw
            };
        }
        // Onto the named set, or the first one (made if none).
        ["add-to-working-set", n, set @ ..] => {
            let id = session(app, n)?;
            add_to_set(app, set.first().copied(), PinTarget::Session(id))?;
        }
        ["add-file-to-working-set", p, rel, set @ ..] => {
            let (id, _) = project(app, p)?;
            add_to_set(
                app,
                set.first().copied(),
                PinTarget::File(id, PathBuf::from(rel)),
            )?;
        }
        _ => return Err(format!("unknown line: {}", w.join(" "))),
    }
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
        ["files", on] => app.dispatch(AppAction::SetFilesOpen(*on == "on")),
        ["side-position", at] => app.dispatch(AppAction::SetSideLeft(*at == "left")),
        ["terminal", on] => app.ui_state.terminal_open = *on == "on",
        ["select-file", p, rel] => {
            let (id, root) = project(app, p)?;
            app.ui_state.files.entry(id).or_default().selected = Some(root.join(rel));
        }
        [
            "set-env" | "set-secret" | "dotenv" | "environment" | "config",
            ..,
        ] => {
            env_step(app, w)?;
        }
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
        ["interrupt", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::Interrupt(id));
        }
        ["approve", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::ApproveDefinition(id));
        }
        ["revoke", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::RevokeApproval(id));
        }
        ["side", tab] => app.dispatch(AppAction::SetSideTab(match *tab {
            "run" => SideTab::Run,
            "notes" => SideTab::Notes,
            _ => SideTab::Files,
        })),
        ["kill", n] => {
            let id = session(app, n)?;
            app.dispatch(AppAction::KillSession(id));
        }
        [first, ..] if REVIEW_LINES.contains(first) => review_step(app, w)?,
        [first, ..] if EXTRA_LINES.contains(first) => working_set_step(app, w)?,
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
        ["config", p] => {
            let (id, _) = project(app, p)?;
            app.ui_state.config_dialog =
                crate::ui::config::ConfigDraft::read(app.core(), app.services(), id);
        }
        _ => return Err(format!("unknown environment line: {}", w.join(" "))),
    }
    Ok(())
}
