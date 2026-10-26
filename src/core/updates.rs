//! The app's own version and the state of an update: checked on a
//! schedule, downloaded on the owner's click, installed on a second
//! click. Versions compare as semantic versions; a release that does
//! not parse, or carries a pre-release tag, never counts as newer.

use std::path::PathBuf;
use std::time::Duration;

use super::action::{AppAction, AppCore, Clock, Effect, Out};
use crate::ports::updates::Release;

/// How long after launch the first check runs: the app settles first.
pub const UPDATE_FIRST_CHECK: Duration = Duration::from_secs(20);
/// How often the check repeats while the app runs.
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(3600);

/// Where an update stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// A newer release, not yet fetched.
    Available(Release),
    Downloading(Release),
    /// Fetched and checked; one click installs it.
    Ready {
        release: Release,
        staged: PathBuf,
    },
    /// The last step failed; `release` is kept so the owner can try
    /// the download again rather than a whole check.
    Failed {
        reason: String,
        release: Option<Release>,
    },
}

impl UpdateState {
    /// A check now would interrupt something.
    fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading(_) | Self::Ready { .. }
        )
    }
}

/// Whether `candidate` is a newer semantic version than `current`. A
/// pre-release (`0.3.0-rc1`) is never offered, and anything that does
/// not parse is not newer, so a malformed tag can never pull an update.
#[must_use]
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (Ok(candidate), Ok(current)) = (
        semver::Version::parse(candidate.trim_start_matches('v')),
        semver::Version::parse(current.trim_start_matches('v')),
    ) else {
        return false;
    };
    candidate.pre.is_empty() && candidate > current
}

impl AppCore {
    /// The running app's version, from the crate unless a test set one.
    #[must_use]
    pub fn version(&self) -> &str {
        self.version_override
            .as_deref()
            .unwrap_or(env!("CARGO_PKG_VERSION"))
    }

    /// Pretend to be this version (tests).
    pub fn set_version(&mut self, version: impl Into<String>) {
        self.version_override = Some(version.into());
    }

    #[must_use]
    pub fn update(&self) -> &UpdateState {
        &self.update
    }

    pub(super) fn update_action(&mut self, action: AppAction, now: Clock, out: &mut Out) {
        match action {
            AppAction::CheckForUpdate => self.check_for_update(now, out),
            AppAction::UpdateChecked(result) => match result {
                Ok(Some(release)) if is_newer(&release.version, self.version()) => {
                    let same = match &self.update {
                        UpdateState::Available(r)
                        | UpdateState::Downloading(r)
                        | UpdateState::Ready { release: r, .. } => r.version == release.version,
                        _ => false,
                    };
                    if !same {
                        self.info(
                            format!(
                                "Switchboard {} is available; update from Settings.",
                                release.version
                            ),
                            now,
                        );
                        self.update = UpdateState::Available(release);
                    }
                }
                Ok(_) => {
                    if !matches!(self.update, UpdateState::Ready { .. }) {
                        self.update = UpdateState::UpToDate;
                    }
                }
                Err(reason) => {
                    if !matches!(self.update, UpdateState::Ready { .. }) {
                        self.update = UpdateState::Failed {
                            reason,
                            release: None,
                        };
                    }
                }
            },
            AppAction::DownloadUpdate => {
                let release = match &self.update {
                    UpdateState::Available(r)
                    | UpdateState::Failed {
                        release: Some(r), ..
                    } => r.clone(),
                    _ => return,
                };
                self.update = UpdateState::Downloading(release.clone());
                out.push(Effect::DownloadUpdate(release));
            }
            AppAction::UpdateDownloaded(result) => {
                let UpdateState::Downloading(release) = &self.update else {
                    return;
                };
                let release = release.clone();
                match result {
                    Ok(staged) => self.update = UpdateState::Ready { release, staged },
                    Err(reason) => {
                        self.error(format!(
                            "Switchboard {} could not be downloaded: {reason}",
                            release.version
                        ));
                        self.update = UpdateState::Failed {
                            reason,
                            release: Some(release),
                        };
                    }
                }
            }
            AppAction::InstallUpdate => {
                if let UpdateState::Ready { staged, .. } = &self.update {
                    out.push(Effect::InstallUpdate(staged.clone()));
                }
            }
            AppAction::UpdateInstallFailed(reason) => {
                let release = match &self.update {
                    UpdateState::Ready { release, .. } => Some(release.clone()),
                    _ => None,
                };
                self.error(format!("The update could not be installed: {reason}"));
                self.update = UpdateState::Failed { reason, release };
            }
            _ => unreachable!("dispatched by `dispatch` itself"),
        }
    }

    fn check_for_update(&mut self, now: Clock, out: &mut Out) {
        if self.update.busy() {
            return;
        }
        // An offered release stays on offer while the check looks for
        // a newer one; the answer compares against it.
        if !matches!(self.update, UpdateState::Available(_)) {
            self.update = UpdateState::Checking;
        }
        self.last_update_check = Some(now.mono);
        out.push(Effect::CheckForUpdate);
    }

    /// The scheduled check: once the app has settled, then hourly,
    /// while the setting allows it. The arithmetic runs from launch and
    /// from the last check of either kind, so turning the setting on
    /// checks at once unless one ran within the hour.
    pub(super) fn update_tick(&mut self, now: Clock, out: &mut Out) {
        if !self.settings.check_for_updates {
            return;
        }
        let due = match self.last_update_check {
            None => now.mono >= UPDATE_FIRST_CHECK,
            Some(at) => now.mono >= at + UPDATE_CHECK_INTERVAL,
        };
        if due {
            self.check_for_update(now, out);
        }
    }
}
