//! Claude Code and Codex launchers: command composition, transcript
//! preflight, Codex rollout discovery. See `ports::agent` and spike 01.
//!
//! Claude Code takes its session id at launch (`--session-id`), so the
//! resume handle is known before spawn. Codex cannot, so its id is
//! discovered afterwards from the newest rollout file whose
//! `session_meta` names the launch cwd.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use uuid::Uuid;

use crate::core::{AgentKind, RecordId, ResumeHandle};
use crate::ports::agent::{AgentLaunch, AgentLauncher};

/// Real launcher. Paths are public so tests can point the provider
/// directories at fixtures.
#[derive(Debug, Clone)]
pub struct Agents {
    pub data_dir: PathBuf,
    /// `claude --settings <this>`; written by `hooks::write_hook_settings`.
    pub hook_settings: PathBuf,
    pub claude_bin: Option<PathBuf>,
    pub codex_bin: Option<PathBuf>,
    /// `~/.claude/projects`: transcripts live at `<slug>/<session id>.jsonl`.
    pub claude_projects: PathBuf,
    /// `~/.codex/sessions`: rollouts live at `YYYY/MM/DD/rollout-*.jsonl`.
    pub codex_sessions: PathBuf,
}

impl Agents {
    /// Finds `claude` and `codex` on `PATH` and in the usual install
    /// locations. Nothing is run.
    #[must_use]
    pub fn detect(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        let home = home_dir();
        Self {
            hook_settings: data_dir.join("claude-hooks.json"),
            data_dir,
            claude_bin: find_binary("claude", &[home.join(".claude/local/claude")]),
            codex_bin: find_binary("codex", &[]),
            claude_projects: home.join(".claude/projects"),
            codex_sessions: home.join(".codex/sessions"),
        }
    }

    fn env(&self, record: RecordId) -> Vec<(String, String)> {
        vec![
            ("SWITCHBOARD_RECORD_ID".into(), record.0.to_string()),
            (
                "SWITCHBOARD_DATA_DIR".into(),
                self.data_dir.to_string_lossy().into_owned(),
            ),
        ]
    }

    fn binary(&self, kind: AgentKind) -> Result<String, String> {
        let bin = match kind {
            AgentKind::ClaudeCode => &self.claude_bin,
            AgentKind::Codex => &self.codex_bin,
        };
        bin.as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} is not installed", kind.label()))
    }

    /// Where Claude Code will write the transcript for a session started
    /// in `cwd`.
    #[must_use]
    pub fn claude_transcript(&self, cwd: &Path, session_id: Uuid) -> PathBuf {
        let cwd = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        self.claude_projects
            .join(claude_slug(&cwd))
            .join(format!("{session_id}.jsonl"))
    }

    /// Searches the sessions tree for `rollout-*-<id>.jsonl`.
    fn find_codex_rollout(&self, rollout_id: &str) -> Option<PathBuf> {
        let suffix = format!("-{rollout_id}.jsonl");
        rollout_files(&self.codex_sessions, None)
            .into_iter()
            .find(|p| p.to_string_lossy().ends_with(&suffix))
    }
}

/// Claude Code's project directory name for a cwd: the absolute path with
/// every `/`, `_`, and `.` replaced by `-` (spike 01, verified against
/// `~/.claude/projects` for 2.1.x).
#[must_use]
pub fn claude_slug(cwd: &Path) -> String {
    cwd.to_string_lossy()
        .chars()
        .map(|c| if matches!(c, '/' | '_' | '.') { '-' } else { c })
        .collect()
}

impl AgentLauncher for Agents {
    fn available(&self, kind: AgentKind) -> bool {
        self.binary(kind).is_ok()
    }

    fn prepare_launch(
        &self,
        kind: AgentKind,
        record: RecordId,
        name: &str,
        cwd: &Path,
    ) -> Result<AgentLaunch, String> {
        let bin = self.binary(kind)?;
        let env = self.env(record);
        match kind {
            AgentKind::ClaudeCode => {
                let session_id = Uuid::new_v4();
                Ok(AgentLaunch {
                    argv: vec![
                        bin,
                        "--session-id".into(),
                        session_id.to_string(),
                        "--name".into(),
                        name.into(),
                        "--settings".into(),
                        self.hook_settings.to_string_lossy().into_owned(),
                    ],
                    env,
                    resume: Some(ResumeHandle::ClaudeCode {
                        session_id,
                        transcript: Some(self.claude_transcript(cwd, session_id)),
                    }),
                })
            }
            AgentKind::Codex => Ok(AgentLaunch {
                argv: vec![bin],
                env,
                resume: None,
            }),
        }
    }

    fn prepare_resume(
        &self,
        handle: &ResumeHandle,
        record: RecordId,
        name: &str,
        _cwd: &Path,
    ) -> Result<AgentLaunch, String> {
        let env = self.env(record);
        match handle {
            ResumeHandle::ClaudeCode { session_id, .. } => Ok(AgentLaunch {
                argv: vec![
                    self.binary(AgentKind::ClaudeCode)?,
                    "--resume".into(),
                    session_id.to_string(),
                    "--name".into(),
                    name.into(),
                    "--settings".into(),
                    self.hook_settings.to_string_lossy().into_owned(),
                ],
                env,
                resume: Some(handle.clone()),
            }),
            ResumeHandle::Codex { rollout_id, .. } => Ok(AgentLaunch {
                argv: vec![
                    self.binary(AgentKind::Codex)?,
                    "resume".into(),
                    rollout_id.clone(),
                ],
                env,
                resume: Some(handle.clone()),
            }),
        }
    }

    fn transcript_exists(&self, handle: &ResumeHandle) -> bool {
        self.transcript_path(handle).is_some_and(|p| p.is_file())
    }

    fn discover(
        &self,
        kind: AgentKind,
        cwd: &Path,
        since: SystemTime,
    ) -> Result<Option<ResumeHandle>, String> {
        if kind != AgentKind::Codex {
            // Claude Code ids are assigned at launch; nothing to discover.
            return Ok(None);
        }
        let want = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let mut found = Vec::new();
        for path in rollout_files(&self.codex_sessions, Some(since)) {
            if let Some(meta) = read_session_meta(&path)
                && meta.cwd == want
            {
                found.push(ResumeHandle::Codex {
                    rollout_id: meta.session_id,
                    transcript: Some(path),
                });
            }
        }
        match found.len() {
            0 => Ok(None),
            1 => Ok(found.pop()),
            _ => Err("ambiguous".into()),
        }
    }

    fn transcript_path(&self, handle: &ResumeHandle) -> Option<PathBuf> {
        match handle {
            ResumeHandle::ClaudeCode { transcript, .. } => transcript.clone(),
            ResumeHandle::Codex {
                rollout_id,
                transcript,
            } => transcript
                .clone()
                .or_else(|| self.find_codex_rollout(rollout_id)),
        }
    }
}

struct SessionMeta {
    session_id: String,
    cwd: PathBuf,
}

/// The first line of a rollout file, when it is a `session_meta` record.
fn read_session_meta(path: &Path) -> Option<SessionMeta> {
    let mut first = String::new();
    BufReader::new(File::open(path).ok()?)
        .read_line(&mut first)
        .ok()?;
    let v: serde_json::Value = serde_json::from_str(&first).ok()?;
    if v["type"] != "session_meta" {
        return None;
    }
    let payload = &v["payload"];
    let session_id = payload["session_id"]
        .as_str()
        .or_else(|| payload["id"].as_str())?
        .to_string();
    let cwd = PathBuf::from(payload["cwd"].as_str()?);
    Some(SessionMeta {
        session_id,
        cwd: cwd.canonicalize().unwrap_or(cwd),
    })
}

/// `rollout-*.jsonl` files under `<root>/YYYY/MM/DD`, optionally only
/// those that came into being at or after `since`.
///
/// A live Codex session keeps appending to its rollout, so the mtime of
/// an earlier session's file can fall inside a later launch's window and
/// bind two records to one conversation. The filter therefore uses the
/// earliest timestamp the file carries: birth time where the filesystem
/// records it (APFS does), else mtime; a file is at least as old as either.
fn rollout_files(root: &Path, since: Option<SystemTime>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![(root.to_path_buf(), 0u8)];
    while let Some((dir, depth)) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if depth < 3 {
                    dirs.push((path, depth + 1));
                }
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
                continue;
            }
            if let Some(since) = since {
                let fresh = entry
                    .metadata()
                    .and_then(|m| born(&m))
                    .is_ok_and(|t| t >= since);
                if !fresh {
                    continue;
                }
            }
            out.push(path);
        }
    }
    out
}

/// When a file came into being: the earlier of its birth time and mtime.
fn born(m: &std::fs::Metadata) -> std::io::Result<SystemTime> {
    let modified = m.modified()?;
    Ok(m.created().map_or(modified, |c| c.min(modified)))
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(|| PathBuf::from("/"), PathBuf::from)
}

/// `which`-style search: `PATH` first, then `extra`, then Homebrew and
/// `~/.local/bin` for launches from a Finder-started app whose `PATH` is
/// the system default.
fn find_binary(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let home = home_dir();
    let path_dirs = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    path_dirs
        .iter()
        .map(|d| d.join(name))
        .chain(extra.iter().cloned())
        .chain([
            PathBuf::from("/opt/homebrew/bin").join(name),
            home.join(".local/bin").join(name),
            PathBuf::from("/usr/local/bin").join(name),
        ])
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn agents(dir: &Path) -> Agents {
        Agents {
            data_dir: dir.join("data"),
            hook_settings: dir.join("data/claude-hooks.json"),
            claude_bin: Some(PathBuf::from("/opt/bin/claude")),
            codex_bin: Some(PathBuf::from("/opt/bin/codex")),
            claude_projects: dir.join("claude-projects"),
            codex_sessions: dir.join("codex-sessions"),
        }
    }

    #[test]
    fn slug_replaces_separators_underscores_and_dots() {
        assert_eq!(
            claude_slug(Path::new(
                "/Users/sully/code_repos/personal/switchboard/spikes/01-claude-resume/work-a"
            )),
            "-Users-sully-code-repos-personal-switchboard-spikes-01-claude-resume-work-a"
        );
        assert_eq!(claude_slug(Path::new("/tmp/a.b_c")), "-tmp-a-b-c");
    }

    #[test]
    fn claude_launch_argv_and_handle() {
        let dir = tempfile::tempdir().unwrap();
        let a = agents(dir.path());
        let record = RecordId::new();
        let cwd = dir.path().join("proj");
        std::fs::create_dir_all(&cwd).unwrap();
        let launch = a
            .prepare_launch(AgentKind::ClaudeCode, record, "my agent", &cwd)
            .unwrap();
        let Some(ResumeHandle::ClaudeCode {
            session_id,
            transcript,
        }) = launch.resume.clone()
        else {
            panic!("expected a Claude handle");
        };
        assert_eq!(
            launch.argv,
            vec![
                "/opt/bin/claude".to_string(),
                "--session-id".into(),
                session_id.to_string(),
                "--name".into(),
                "my agent".into(),
                "--settings".into(),
                a.hook_settings.to_string_lossy().into_owned(),
            ]
        );
        assert_eq!(
            launch.env,
            vec![
                ("SWITCHBOARD_RECORD_ID".to_string(), record.0.to_string()),
                (
                    "SWITCHBOARD_DATA_DIR".to_string(),
                    a.data_dir.to_string_lossy().into_owned()
                ),
            ]
        );
        let real_cwd = cwd.canonicalize().unwrap();
        let expected = a
            .claude_projects
            .join(claude_slug(&real_cwd))
            .join(format!("{session_id}.jsonl"));
        assert_eq!(transcript.as_deref(), Some(expected.as_path()));

        let handle = launch.resume.unwrap();
        assert!(!a.transcript_exists(&handle));
        std::fs::create_dir_all(expected.parent().unwrap()).unwrap();
        std::fs::write(&expected, "{}\n").unwrap();
        assert!(a.transcript_exists(&handle));
        assert_eq!(a.transcript_path(&handle), Some(expected));
    }

    #[test]
    fn resume_argv() {
        let dir = tempfile::tempdir().unwrap();
        let a = agents(dir.path());
        let record = RecordId::new();
        let id = Uuid::new_v4();
        let claude = a
            .prepare_resume(
                &ResumeHandle::ClaudeCode {
                    session_id: id,
                    transcript: None,
                },
                record,
                "n",
                dir.path(),
            )
            .unwrap();
        assert_eq!(
            claude.argv,
            vec![
                "/opt/bin/claude".to_string(),
                "--resume".into(),
                id.to_string(),
                "--name".into(),
                "n".into(),
                "--settings".into(),
                a.hook_settings.to_string_lossy().into_owned(),
            ]
        );
        let codex = a
            .prepare_resume(
                &ResumeHandle::Codex {
                    rollout_id: "abc".into(),
                    transcript: None,
                },
                record,
                "n",
                dir.path(),
            )
            .unwrap();
        assert_eq!(
            codex.argv,
            vec!["/opt/bin/codex".to_string(), "resume".into(), "abc".into()]
        );
        assert_eq!(codex.env[0].1, record.0.to_string());

        let launch = a
            .prepare_launch(AgentKind::Codex, record, "n", dir.path())
            .unwrap();
        assert_eq!(launch.argv, vec!["/opt/bin/codex".to_string()]);
        assert!(launch.resume.is_none());
    }

    #[test]
    fn missing_binary_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = agents(dir.path());
        a.codex_bin = None;
        assert!(!a.available(AgentKind::Codex));
        assert!(a.available(AgentKind::ClaudeCode));
        assert!(
            a.prepare_launch(AgentKind::Codex, RecordId::new(), "n", dir.path())
                .is_err()
        );
    }

    fn write_rollout(root: &Path, day: &str, id: &str, cwd: &Path) -> PathBuf {
        let dir = root.join(day);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("rollout-2026-09-06T10-00-00-{id}.jsonl"));
        let mut f = File::create(&path).unwrap();
        let meta = serde_json::json!({
            "timestamp": "2026-09-06T10:00:00.000Z",
            "ordinal": 0,
            "type": "session_meta",
            "payload": {"session_id": id, "id": id, "cwd": cwd, "originator": "codex-tui"}
        });
        writeln!(f, "{meta}").unwrap();
        writeln!(f, "{{\"type\":\"turn_context\",\"payload\":{{}}}}").unwrap();
        path
    }

    #[test]
    fn discovers_the_codex_rollout_for_a_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let a = agents(dir.path());
        let cwd = dir.path().join("work");
        let other = dir.path().join("other");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        let old = write_rollout(&a.codex_sessions, "2026/09/05", "old-id", &cwd);
        let since = SystemTime::now();
        // Give the old file an mtime before `since` even on coarse clocks.
        let earlier = since - std::time::Duration::from_secs(60);
        File::open(&old).unwrap().set_modified(earlier).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        write_rollout(&a.codex_sessions, "2026/09/06", "elsewhere", &other);
        assert_eq!(a.discover(AgentKind::Codex, &cwd, since).unwrap(), None);

        let mine = write_rollout(&a.codex_sessions, "2026/09/06", "mine-id", &cwd);
        let handle = a.discover(AgentKind::Codex, &cwd, since).unwrap().unwrap();
        assert_eq!(
            handle,
            ResumeHandle::Codex {
                rollout_id: "mine-id".into(),
                transcript: Some(mine.clone()),
            }
        );
        assert!(a.transcript_exists(&handle));
        let bare = ResumeHandle::Codex {
            rollout_id: "mine-id".into(),
            transcript: None,
        };
        assert_eq!(a.transcript_path(&bare), Some(mine));
        assert!(a.transcript_exists(&bare));
        assert!(!a.transcript_exists(&ResumeHandle::Codex {
            rollout_id: "nope".into(),
            transcript: None,
        }));

        write_rollout(&a.codex_sessions, "2026/09/06", "twin-id", &cwd);
        assert_eq!(
            a.discover(AgentKind::Codex, &cwd, since),
            Err("ambiguous".into())
        );
        assert_eq!(a.discover(AgentKind::ClaudeCode, &cwd, since), Ok(None));
    }
}
