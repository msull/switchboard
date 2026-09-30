//! Pull requests on Bitbucket Cloud, read through its REST API with
//! `curl`. Only reads: Dispatch never opens, approves or merges a PR
//! here. The account token comes from `BITBUCKET_EMAIL` and
//! `BITBUCKET_API_TOKEN` in the runner's environment, else from
//! `<data dir>/env` (`NAME=value` lines, mode 0600); it goes to `curl`
//! on stdin, never on a command line or into a log.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::github::{Checks, PullRequest, PullRequests};

const API_ROOT: &str = "https://api.bitbucket.org/2.0";

/// `workspace/repo` from the ways a Bitbucket remote is written, or
/// `None` for a remote on another host.
#[must_use]
pub fn bitbucket_repo(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@bitbucket.org:")
        .or_else(|| url.strip_prefix("ssh://git@bitbucket.org/"))
        .or_else(|| {
            url.strip_prefix("https://")
                .and_then(|r| r.split_once("bitbucket.org/").map(|(_, r)| r))
        })
        .or_else(|| url.strip_prefix("bitbucket.org/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (ws, name) = (parts.next()?, parts.next()?);
    if ws.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{ws}/{name}"))
}

/// The Bitbucket Cloud API as `curl` reaches it.
#[derive(Debug)]
pub struct Bitbucket {
    /// Where credentials are read from when the environment has none.
    pub env_file: PathBuf,
}

impl Bitbucket {
    #[must_use]
    pub fn new(env_file: PathBuf) -> Self {
        Self { env_file }
    }

    /// `(email, token)`, from the environment or the env file.
    fn credentials(&self) -> Result<(String, String)> {
        let from_env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        if let (Some(email), Some(token)) =
            (from_env("BITBUCKET_EMAIL"), from_env("BITBUCKET_API_TOKEN"))
        {
            return Ok((email, token));
        }
        let text = std::fs::read_to_string(&self.env_file).with_context(|| {
            format!(
                "BITBUCKET_EMAIL and BITBUCKET_API_TOKEN are not in the environment, and {} cannot be read",
                self.env_file.display()
            )
        })?;
        let mut email = None;
        let mut token = None;
        for line in text.lines() {
            let line = line.trim().trim_start_matches("export ");
            if let Some((k, v)) = line.split_once('=') {
                let v = v.trim().trim_matches('"').trim_matches('\'').to_owned();
                match k.trim() {
                    "BITBUCKET_EMAIL" => email = Some(v),
                    "BITBUCKET_API_TOKEN" => token = Some(v),
                    _ => {}
                }
            }
        }
        match (email, token) {
            (Some(e), Some(t)) if !e.is_empty() && !t.is_empty() => Ok((e, t)),
            _ => bail!(
                "{} names no BITBUCKET_EMAIL and BITBUCKET_API_TOKEN",
                self.env_file.display()
            ),
        }
    }

    /// GET a URL as the account; the response body.
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        let (email, token) = self.credentials()?;
        let mut child = Command::new("curl")
            .args([
                "-sS",
                "--fail-with-body",
                "--max-time",
                "30",
                "--config",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("run curl")?;
        {
            let mut stdin = child.stdin.take().context("curl's stdin")?;
            // curl's config quoting: backslashes and quotes in the value
            // are escaped, and nothing here is logged.
            let user = format!("{email}:{token}")
                .replace('\\', "\\\\")
                .replace('"', "\\\"");
            writeln!(stdin, "url = \"{url}\"")?;
            writeln!(stdin, "user = \"{user}\"")?;
            writeln!(stdin, "header = \"Accept: application/json\"")?;
        }
        let out = child.wait_with_output().context("wait for curl")?;
        if !out.status.success() {
            let body = String::from_utf8_lossy(&out.stdout);
            let detail = serde_json::from_str::<ApiError>(&body)
                .ok()
                .and_then(|e| e.error.map(|e| e.message))
                .unwrap_or_else(|| String::from_utf8_lossy(&out.stderr).trim().to_owned());
            bail!("bitbucket {}: {detail}", url.trim_start_matches(API_ROOT));
        }
        Ok(out.stdout)
    }
}

#[derive(Deserialize)]
struct ApiError {
    error: Option<ApiMessage>,
}

#[derive(Deserialize)]
struct ApiMessage {
    #[serde(default)]
    message: String,
}

#[derive(Deserialize)]
struct Page<T> {
    #[serde(default = "Vec::new")]
    values: Vec<T>,
}

#[derive(Deserialize)]
struct PrRow {
    id: u64,
    #[serde(default)]
    state: String,
    #[serde(default)]
    source: Source,
    #[serde(default)]
    links: Links,
}

#[derive(Deserialize, Default)]
struct Source {
    #[serde(default)]
    commit: Option<Commit>,
}

#[derive(Deserialize)]
struct Commit {
    #[serde(default)]
    hash: String,
}

#[derive(Deserialize, Default)]
struct Links {
    #[serde(default)]
    html: Option<Href>,
}

#[derive(Deserialize)]
struct Href {
    #[serde(default)]
    href: String,
}

#[derive(Deserialize)]
struct StatusRow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    state: String,
}

/// The PR for a branch from a `pullrequests` page: an open one first,
/// else the newest of the rest. Bitbucket reports the source commit as
/// a short hash.
pub(crate) fn parse_prs(json: &[u8]) -> Result<Option<PullRequest>> {
    let page: Page<PrRow> = serde_json::from_slice(json).context("parse bitbucket's json")?;
    let row = page
        .values
        .iter()
        .find(|r| r.state == "OPEN")
        .or_else(|| page.values.iter().max_by_key(|r| r.id));
    Ok(row.map(|r| PullRequest {
        number: r.id,
        url: r
            .links
            .html
            .as_ref()
            .map(|h| h.href.clone())
            .unwrap_or_default(),
        head: r
            .source
            .commit
            .as_ref()
            .map(|c| c.hash.clone())
            .unwrap_or_default(),
        state: match r.state.as_str() {
            "OPEN" => "open".to_owned(),
            "MERGED" => "merged".to_owned(),
            _ => "closed".to_owned(),
        },
    }))
}

/// The commit statuses on a PR's head, taken together.
pub(crate) fn parse_statuses(json: &[u8]) -> Result<Checks> {
    let page: Page<StatusRow> = serde_json::from_slice(json).context("parse bitbucket's json")?;
    if page.values.is_empty() {
        return Ok(Checks::None);
    }
    let failed: Vec<String> = page
        .values
        .iter()
        .filter(|s| matches!(s.state.as_str(), "FAILED" | "STOPPED"))
        .map(|s| s.name.clone())
        .collect();
    if !failed.is_empty() {
        return Ok(Checks::Failed(failed));
    }
    if page.values.iter().any(|s| s.state == "INPROGRESS") {
        return Ok(Checks::Pending);
    }
    Ok(Checks::Passed)
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

impl PullRequests for Bitbucket {
    fn find(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>> {
        let q = encode(&format!("source.branch.name=\"{branch}\""));
        let url = format!(
            "{API_ROOT}/repositories/{repo}/pullrequests?q={q}&state=OPEN&state=MERGED&state=DECLINED&state=SUPERSEDED&pagelen=50"
        );
        parse_prs(&self.get(&url)?)
    }

    fn checks(&self, repo: &str, number: u64) -> Result<Checks> {
        let url =
            format!("{API_ROOT}/repositories/{repo}/pullrequests/{number}/statuses?pagelen=100");
        parse_statuses(&self.get(&url)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bitbucket_remote_is_read_in_every_spelling_and_others_are_not() {
        for url in [
            "git@bitbucket.org:cainfosec/delta-backend.git",
            "https://bitbucket.org/cainfosec/delta-backend.git",
            "https://sully@bitbucket.org/cainfosec/delta-backend",
            "ssh://git@bitbucket.org/cainfosec/delta-backend.git",
        ] {
            assert_eq!(
                bitbucket_repo(url).as_deref(),
                Some("cainfosec/delta-backend")
            );
        }
        assert_eq!(bitbucket_repo("git@github.com:msull/switchboard.git"), None);
    }

    #[test]
    fn a_pull_request_page_gives_the_open_one_with_its_short_head() {
        let json = br#"{"values":[
            {"id":3,"state":"DECLINED","source":{"commit":{"hash":"aaaaaaaaaaaa"}},"links":{"html":{"href":"https://bitbucket.org/w/r/pull-requests/3"}}},
            {"id":7,"state":"OPEN","source":{"commit":{"hash":"fa62f3f78577"}},"links":{"html":{"href":"https://bitbucket.org/w/r/pull-requests/7"}}}
        ]}"#;
        let pr = parse_prs(json).unwrap().unwrap();
        assert_eq!(
            (pr.number, pr.state.as_str(), pr.head.as_str()),
            (7, "open", "fa62f3f78577")
        );
        assert_eq!(pr.url, "https://bitbucket.org/w/r/pull-requests/7");
        let merged = br#"{"values":[{"id":2,"state":"MERGED","source":{"commit":{"hash":"bb"}}}]}"#;
        assert_eq!(parse_prs(merged).unwrap().unwrap().state, "merged");
        assert!(parse_prs(br#"{"values":[]}"#).unwrap().is_none());
    }

    #[test]
    fn statuses_read_as_none_pending_passed_or_failed_by_name() {
        assert_eq!(parse_statuses(br#"{"values":[]}"#).unwrap(), Checks::None);
        let pending = br#"{"values":[{"name":"build","state":"INPROGRESS"},{"name":"lint","state":"SUCCESSFUL"}]}"#;
        assert_eq!(parse_statuses(pending).unwrap(), Checks::Pending);
        let failed = br#"{"values":[{"name":"build","state":"FAILED"},{"name":"lint","state":"SUCCESSFUL"}]}"#;
        assert_eq!(
            parse_statuses(failed).unwrap(),
            Checks::Failed(vec!["build".into()])
        );
        let passed = br#"{"values":[{"name":"build","state":"SUCCESSFUL"}]}"#;
        assert_eq!(parse_statuses(passed).unwrap(), Checks::Passed);
    }

    #[test]
    fn credentials_come_from_the_env_file_when_the_environment_has_none() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("env");
        std::fs::write(
            &file,
            "export BITBUCKET_EMAIL=\"me@example.com\"\nBITBUCKET_API_TOKEN=tok'en\n",
        )
        .unwrap();
        let bb = Bitbucket::new(file);
        // The test environment has no Bitbucket variables set.
        assert!(std::env::var("BITBUCKET_API_TOKEN").is_err());
        let (email, token) = bb.credentials().unwrap();
        assert_eq!(
            (email.as_str(), token.as_str()),
            ("me@example.com", "tok'en")
        );
        let none = Bitbucket::new(dir.path().join("missing"));
        assert!(
            none.credentials()
                .unwrap_err()
                .to_string()
                .contains("cannot be read")
        );
    }
}
