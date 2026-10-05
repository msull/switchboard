//! Issues and pull requests, through the `gh` CLI. Only reads: Dispatch
//! never comments on, labels or closes an issue, and never merges.

use std::process::Command;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::ticket::SourceSnapshot;

pub trait Issues: Send {
    fn fetch(&self, repo: &str, number: u64, now_ms: u64) -> Result<SourceSnapshot>;
}

/// The `gh` CLI: `gh issue view` for issues, `gh pr` for pull requests
/// and their checks.
#[derive(Debug, Default)]
pub struct Gh;

/// A pull request as a gate reads it: which commit it proposes, and
/// whether it is still open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub number: u64,
    pub url: String,
    /// The head commit the PR is at now.
    pub head: String,
    /// `open`, `merged` or `closed`.
    pub state: String,
    /// Whether it can merge as it stands: `clean`, `conflicting`, or
    /// `None` where the provider does not say.
    pub mergeable: Option<String>,
    /// The branch it comes from.
    pub branch: String,
    /// The branch it goes into.
    pub base: String,
    pub title: String,
}

/// What a PR's checks say, taken together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checks {
    /// The repository reports no checks at all for the PR.
    None,
    Pending,
    Passed,
    /// The names of the checks that failed.
    Failed(Vec<String>),
}

/// Pull requests on a hosted repository, found by branch.
pub trait PullRequests: Send {
    /// The PR whose head is `branch`, if one exists in any state.
    fn find(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>>;
    /// The PR with this number; an error when there is none.
    fn by_number(&self, repo: &str, number: u64) -> Result<PullRequest>;
    /// The checks on a PR, as a whole.
    fn checks(&self, repo: &str, number: u64) -> Result<Checks>;
}

/// `owner/name` from the ways a GitHub remote is written, or `None`
/// for a remote on another host.
#[must_use]
pub fn github_repo(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("git@github.com:")
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("https://github.com/"))
        .or_else(|| url.strip_prefix("github.com/"))?;
    let rest = rest.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = rest.split('/');
    let (owner, name) = (parts.next()?, parts.next()?);
    if owner.is_empty() || name.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{name}"))
}

#[derive(Deserialize)]
struct PrRow {
    number: u64,
    #[serde(default)]
    url: String,
    #[serde(rename = "headRefOid", default)]
    head: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    mergeable: String,
    #[serde(rename = "headRefName", default)]
    branch: String,
    #[serde(rename = "baseRefName", default)]
    base: String,
    #[serde(default)]
    title: String,
}

impl PrRow {
    fn pull_request(&self) -> PullRequest {
        PullRequest {
            number: self.number,
            url: self.url.clone(),
            head: self.head.clone(),
            state: self.state.to_ascii_lowercase(),
            mergeable: match self.mergeable.as_str() {
                "CONFLICTING" => Some("conflicting".to_owned()),
                "MERGEABLE" => Some("clean".to_owned()),
                _ => None,
            },
            branch: self.branch.clone(),
            base: self.base.clone(),
            title: self.title.clone(),
        }
    }
}

const PR_FIELDS: &str = "number,url,headRefOid,state,mergeable,headRefName,baseRefName,title";

#[derive(Deserialize)]
struct CheckRow {
    #[serde(default)]
    name: String,
    #[serde(default)]
    bucket: String,
}

impl PullRequests for Gh {
    fn find(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>> {
        let out = Command::new("gh")
            .args([
                "pr", "list", "--repo", repo, "--head", branch, "--state", "all",
            ])
            .args(["--json", PR_FIELDS])
            .output()
            .context("run gh")?;
        if !out.status.success() {
            bail!(
                "gh pr list --repo {repo} --head {branch}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let rows: Vec<PrRow> = serde_json::from_slice(&out.stdout).context("parse gh's json")?;
        // An open PR first; otherwise the newest of the rest.
        let row = rows
            .iter()
            .find(|r| r.state == "OPEN")
            .or_else(|| rows.iter().max_by_key(|r| r.number));
        Ok(row.map(PrRow::pull_request))
    }

    fn by_number(&self, repo: &str, number: u64) -> Result<PullRequest> {
        let out = Command::new("gh")
            .args(["pr", "view", &number.to_string(), "--repo", repo])
            .args(["--json", PR_FIELDS])
            .output()
            .context("run gh")?;
        if !out.status.success() {
            bail!(
                "gh pr view {number} --repo {repo}: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        let row: PrRow = serde_json::from_slice(&out.stdout).context("parse gh's json")?;
        Ok(row.pull_request())
    }

    fn checks(&self, repo: &str, number: u64) -> Result<Checks> {
        let out = Command::new("gh")
            .args(["pr", "checks", &number.to_string(), "--repo", repo])
            .args(["--json", "name,bucket"])
            .output()
            .context("run gh")?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Failing checks make `gh` exit nonzero too, so only the message
        // tells a repository without CI from a lookup failure.
        if stderr.contains("no checks reported") {
            return Ok(Checks::None);
        }
        let rows: Vec<CheckRow> = match serde_json::from_slice(&out.stdout) {
            Ok(rows) => rows,
            Err(_) if !out.status.success() => {
                bail!("gh pr checks {number} --repo {repo}: {}", stderr.trim())
            }
            Err(e) => return Err(e).context("parse gh's json"),
        };
        if rows.is_empty() {
            return Ok(Checks::None);
        }
        let failed: Vec<String> = rows
            .iter()
            .filter(|r| matches!(r.bucket.as_str(), "fail" | "cancel"))
            .map(|r| r.name.clone())
            .collect();
        if !failed.is_empty() {
            return Ok(Checks::Failed(failed));
        }
        if rows.iter().any(|r| r.bucket == "pending") {
            return Ok(Checks::Pending);
        }
        Ok(Checks::Passed)
    }
}

/// Pull requests a test hands out, shared with the test so it can move
/// them while the runner runs.
#[derive(Debug, Default)]
pub struct FakePullRequests {
    /// `(repo, branch)` to the PR there.
    pub prs: Vec<(String, String, PullRequest)>,
    /// `(repo, number)` to its checks; absent means `Checks::None`.
    pub checks: Vec<(String, u64, Checks)>,
    /// Every lookup fails with this message.
    pub fail: Option<String>,
    /// How many times a PR was looked for.
    pub looked: u32,
}

impl PullRequests for Arc<Mutex<FakePullRequests>> {
    fn find(&self, repo: &str, branch: &str) -> Result<Option<PullRequest>> {
        let mut fake = self.lock().unwrap();
        fake.looked += 1;
        if let Some(why) = &fake.fail {
            bail!("{why}");
        }
        Ok(fake
            .prs
            .iter()
            .find(|(r, b, _)| r == repo && b == branch)
            .map(|(_, _, pr)| pr.clone()))
    }

    fn by_number(&self, repo: &str, number: u64) -> Result<PullRequest> {
        let fake = self.lock().unwrap();
        if let Some(why) = &fake.fail {
            bail!("{why}");
        }
        fake.prs
            .iter()
            .find(|(r, _, pr)| r == repo && pr.number == number)
            .map(|(_, _, pr)| pr.clone())
            .with_context(|| format!("no pull request {repo}#{number}"))
    }

    fn checks(&self, repo: &str, number: u64) -> Result<Checks> {
        let fake = self.lock().unwrap();
        if let Some(why) = &fake.fail {
            bail!("{why}");
        }
        Ok(fake
            .checks
            .iter()
            .find(|(r, n, _)| r == repo && *n == number)
            .map_or(Checks::None, |(_, _, c)| c.clone()))
    }
}

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
            taken_by: None,
            pull_requests: Vec::new(),
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
            taken_by: None,
            pull_requests: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::github_repo;

    #[test]
    fn a_github_remote_is_read_in_every_spelling_and_others_are_not() {
        for url in [
            "git@github.com:example-org/widgets.git",
            "https://github.com/example-org/widgets",
            "https://github.com/example-org/widgets.git",
            "ssh://git@github.com/example-org/widgets.git",
        ] {
            assert_eq!(github_repo(url).as_deref(), Some("example-org/widgets"));
        }
        assert_eq!(
            github_repo("git@bitbucket.org:example-co/orchard-frontend.git"),
            None
        );
        assert_eq!(github_repo("https://github.com/msull"), None);
    }
}
