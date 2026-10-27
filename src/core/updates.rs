//! The app's own version and the state of an update: checked on a
//! schedule, downloaded on the owner's click, installed on a second
//! click. Versions compare as semantic versions; a release that does
//! not parse, or carries a pre-release tag, never counts as newer.

use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use super::action::{AppAction, AppCore, Clock, Effect, Out};
use crate::ports::updates::{CheckOutcome, Release};

/// How long after every launch the first check waits: the app settles
/// first, even when the day has long passed.
pub const UPDATE_FIRST_CHECK: Duration = Duration::from_secs(30);
/// How long after the last answer the scheduled check runs again.
pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_hours(24);
/// The most the period is stretched, so copies that last answered at
/// different times do not ask at the same minute.
pub const UPDATE_JITTER_MAX: Duration = Duration::from_mins(30);
/// How soon a scheduled check that got no HTTP answer at all is tried
/// again. A host that answered with an error is not asked again until
/// the period has passed.
pub const UPDATE_RETRY: Duration = Duration::from_hours(1);

/// The stretch added to the period after an answer at `checked_at`
/// (Unix seconds): a hash of that second, so it is drawn once per
/// answer and neither a relaunch nor a wake moves it.
#[must_use]
pub fn update_jitter(checked_at: u64) -> Duration {
    // splitmix64's finaliser: a cheap hash with every input bit mixed.
    let mut z = checked_at.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    Duration::from_secs(z % (UPDATE_JITTER_MAX.as_secs() + 1))
}

/// `now` as Unix seconds; a clock before the epoch reads as 0, so the
/// last answer looks long past and a check is due rather than never.
fn unix_secs(now: Clock) -> u64 {
    now.wall
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Who asked for the check that is out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpdateCheck {
    /// The tick; its failure stays quiet.
    Scheduled,
    /// The owner's click; its result always shows.
    Manual,
}

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
            AppAction::CheckForUpdate => self.check_for_update(true, out),
            AppAction::UpdateChecked(result) => {
                let outcome = match result {
                    Ok(release) => CheckOutcome::Answer {
                        release,
                        etag: None,
                    },
                    Err(reason) => CheckOutcome::NoAnswer(reason),
                };
                self.update_answered(outcome, now, out);
            }
            AppAction::UpdateAnswered(outcome) => self.update_answered(outcome, now, out),
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

    fn check_for_update(&mut self, manual: bool, out: &mut Out) {
        // A scheduled check shows no state, so `busy` alone would not
        // see one that is out.
        if let Some(out_now) = self.update_inflight {
            // A click while a scheduled check is out takes it over, so
            // the owner sees its result.
            if manual && out_now == UpdateCheck::Scheduled {
                self.update_inflight = Some(UpdateCheck::Manual);
                if !matches!(self.update, UpdateState::Available(_)) {
                    self.update = UpdateState::Checking;
                }
            }
            return;
        }
        if self.update.busy() {
            return;
        }
        // An offered release stays on offer while the check looks for
        // a newer one; the answer compares against it. A scheduled
        // check shows nothing until it has something to offer.
        if manual && !matches!(self.update, UpdateState::Available(_)) {
            self.update = UpdateState::Checking;
        }
        self.update_inflight = Some(if manual {
            UpdateCheck::Manual
        } else {
            UpdateCheck::Scheduled
        });
        out.push(Effect::CheckForUpdate);
    }

    /// What the release host answered. Only a manual check's failure
    /// shows; a scheduled one leaves the state, and any offer, as it
    /// was.
    fn update_answered(&mut self, outcome: CheckOutcome, now: Clock, out: &mut Out) {
        // An answer with no check out is shown as a click's would be.
        let manual =
            self.update_inflight.take().unwrap_or(UpdateCheck::Manual) == UpdateCheck::Manual;
        let at = unix_secs(now);
        match outcome {
            CheckOutcome::Answer { release, etag } => {
                self.update_retry_at = None;
                // The `ETag` is kept only with the release it vouches
                // for, so a 304 always has something to compare.
                let (etag, kept) = match (etag, release.clone()) {
                    (Some(etag), Some(release)) => (Some(etag), Some(release)),
                    _ => (None, None),
                };
                self.update_settings(out, |s| {
                    s.update_check.checked_at = Some(at);
                    s.update_check.etag = etag;
                    s.update_check.release = kept;
                });
                self.judge_release(release, now);
            }
            CheckOutcome::NotModified => {
                self.update_retry_at = None;
                if let Some(release) = self.settings.update_check.release.clone() {
                    self.update_settings(out, |s| s.update_check.checked_at = Some(at));
                    self.judge_release(Some(release), now);
                } else {
                    // Only a hand-edited file gets here; the next
                    // request goes without the `ETag`.
                    self.update_settings(out, |s| s.update_check.etag = None);
                    if manual {
                        self.update_failed(
                            "the release host reported no change, but there is no release to compare"
                                .into(),
                        );
                    }
                }
            }
            CheckOutcome::Http { status, detail } => {
                // The host answered: ask again only once the period has
                // passed, whatever the error, and a relaunch does not
                // ask sooner.
                self.update_retry_at = None;
                self.update_settings(out, |s| s.update_check.checked_at = Some(at));
                if manual {
                    self.update_failed(format!("the release host answered {status}: {detail}"));
                }
            }
            CheckOutcome::NoAnswer(reason) => {
                if manual {
                    self.update_failed(reason);
                } else {
                    self.update_retry_at = Some(now.wall + UPDATE_RETRY);
                }
            }
        }
    }

    /// Offer `release` when it is newer than the running app, once per
    /// version; otherwise the app is up to date.
    fn judge_release(&mut self, release: Option<Release>, now: Clock) {
        match release {
            Some(release) if is_newer(&release.version, self.version()) => {
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
            _ => {
                if !matches!(self.update, UpdateState::Ready { .. }) {
                    self.update = UpdateState::UpToDate;
                }
            }
        }
    }

    /// A manual check that failed: the reason shows, and an offered
    /// release stays so its download can still be tried.
    fn update_failed(&mut self, reason: String) {
        let release = match &self.update {
            UpdateState::Ready { .. } | UpdateState::Downloading(_) => return,
            UpdateState::Available(r)
            | UpdateState::Failed {
                release: Some(r), ..
            } => Some(r.clone()),
            _ => None,
        };
        self.update = UpdateState::Failed { reason, release };
    }

    /// The release the last check described, offered again after a
    /// relaunch while it is still newer. The owner was told when it
    /// was first found, so no notice.
    pub(super) fn restore_update_offer(&mut self) {
        if self.update != UpdateState::Idle {
            return;
        }
        if let Some(release) = &self.settings.update_check.release
            && is_newer(&release.version, self.version())
        {
            self.update = UpdateState::Available(release.clone());
        }
    }

    /// Whether the scheduled check is due: never in the first moments
    /// after a launch; then once the retry after an unanswered check
    /// comes round, or else once the period plus its jitter has passed
    /// since the last answer. All on the wall clock, so a sleep or a
    /// relaunch is seen on the next tick. A time ahead of the clock
    /// means the clock was set back, and the check is due.
    fn update_due(&self, now: Clock) -> bool {
        if now.mono < UPDATE_FIRST_CHECK {
            return false;
        }
        if let Some(retry) = self.update_retry_at {
            return now.wall >= retry || retry > now.wall + UPDATE_RETRY;
        }
        let wall = unix_secs(now);
        match self.settings.update_check.checked_at {
            Some(at) if at <= wall => {
                let next = Duration::from_secs(at) + UPDATE_CHECK_INTERVAL + update_jitter(at);
                Duration::from_secs(wall) >= next
            }
            _ => true,
        }
    }

    /// The scheduled check, while the setting allows it. It counts from
    /// the last answer of either kind, so turning the setting on checks
    /// at once unless one was answered within the period.
    pub(super) fn update_tick(&mut self, now: Clock, out: &mut Out) {
        if !self.settings.check_for_updates {
            return;
        }
        if self.update_due(now) {
            self.check_for_update(false, out);
        }
    }
}
