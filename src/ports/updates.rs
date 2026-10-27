//! The app's own updates: what a newer release is, and how one is
//! fetched, checked and put in place of the running bundle. The source
//! is GitHub Releases outside the tests; the core only compares
//! versions and moves the state.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One published release newer than the running app, as the source
/// describes it: the version (no `v`), its notes, when it was published,
/// and where its bundle and the bundle's checksum are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub asset_url: String,
    pub asset_name: String,
    pub digest_url: Option<String>,
    /// The RFC 3339 time GitHub gives as `published_at`, as sent.
    pub published_at: Option<String>,
}

/// What one request to the release host came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// 200 (or 404, as `release: None`). `etag` is the reply's header, if any.
    Answer {
        release: Option<Release>,
        etag: Option<String>,
    },
    /// 304: the release the `ETag` was sent for is still the newest.
    NotModified,
    /// No HTTP answer at all: DNS, connect, TLS, timeout, a dropped link.
    NoAnswer(String),
    /// An HTTP answer that is no release: 403, 429, 5xx, a refused redirect,
    /// a reply over the cap or not JSON.
    Http { status: u16, detail: String },
}

/// Where releases come from and how one is installed. Every method
/// may block; the shell runs `latest` and `download` on a thread.
pub trait UpdateSource: Send + Sync {
    /// The newest published release, or `None` when nothing is
    /// published; a release that is not newer is still returned, and
    /// the core decides.
    fn latest(&self) -> Result<Option<Release>, String>;
    /// The newest release as `latest` finds it, sent with the `ETag` of
    /// the last answer when there is one, and with the kind of failure
    /// kept so the core can tell a host that answered from one that did
    /// not. A source without conditional requests needs only `latest`.
    fn check(&self, etag: Option<&str>) -> CheckOutcome {
        let _ = etag;
        match self.latest() {
            Ok(release) => CheckOutcome::Answer {
                release,
                etag: None,
            },
            Err(reason) => CheckOutcome::NoAnswer(reason),
        }
    }
    /// Fetch the release's bundle, check it against its checksum and
    /// its signature, and leave it staged; the path to the staged
    /// bundle comes back.
    fn download(&self, release: &Release) -> Result<PathBuf, String>;
    /// Arrange for the staged bundle to replace the running one once
    /// the process exits, and relaunch it. Returns once that is
    /// arranged; the caller then quits.
    fn install(&self, staged: &Path) -> Result<(), String>;
}
