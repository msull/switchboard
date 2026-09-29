//! Issues, through the `gh` CLI. Only reads: Dispatch never comments on,
//! labels or closes an issue.

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::ticket::SourceSnapshot;

pub trait Issues: Send {
    fn fetch(&self, repo: &str, number: u64, now_ms: u64) -> Result<SourceSnapshot>;
}

/// `gh issue view <n> --repo <repo> --json ...`.
#[derive(Debug, Default)]
pub struct Gh;

#[derive(Deserialize)]
struct Issue {
    number: u64,
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

impl Issues for Gh {
    fn fetch(&self, repo: &str, number: u64, now_ms: u64) -> Result<SourceSnapshot> {
        let out = Command::new("gh")
            .args(["issue", "view", &number.to_string(), "--repo", repo])
            .args(["--json", "number,title,body,url,labels"])
            .output()
            .context("run gh")?;
        if !out.status.success() {
            bail!(
                "gh issue view {number} --repo {repo}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let issue: Issue = serde_json::from_slice(&out.stdout).context("parse gh's json")?;
        Ok(SourceSnapshot {
            kind: "github".into(),
            identity: format!("{repo}#{}", issue.number),
            number: Some(issue.number),
            title: issue.title,
            body: issue.body,
            url: Some(issue.url),
            labels: issue.labels.into_iter().map(|l| l.name).collect(),
            taken_at_ms: now_ms,
        })
    }
}

/// Issues a test hands out.
#[derive(Debug, Default)]
pub struct FakeIssues {
    pub issues: Vec<(String, u64, String, String)>,
}

impl Issues for FakeIssues {
    fn fetch(&self, repo: &str, number: u64, now_ms: u64) -> Result<SourceSnapshot> {
        let (_, _, title, body) = self
            .issues
            .iter()
            .find(|(r, n, _, _)| r == repo && *n == number)
            .with_context(|| format!("no issue {repo}#{number}"))?;
        Ok(SourceSnapshot {
            kind: "github".into(),
            identity: format!("{repo}#{number}"),
            number: Some(number),
            title: title.clone(),
            body: body.clone(),
            url: Some(format!("https://github.com/{repo}/issues/{number}")),
            labels: vec!["dispatch".into()],
            taken_at_ms: now_ms,
        })
    }
}
