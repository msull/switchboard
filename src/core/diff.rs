//! A changed file's diff on the ticket page's Changes tab: which range
//! each held diff answered, when a read is due, and which stretches of
//! unchanged lines the page folds. Diffs are a cache of git's objects,
//! never saved; after a relaunch a click reads one again.

use std::ops::Range;
use std::path::PathBuf;

use super::action::{AppAction, AppCore, Effect, Out};
use crate::ports::changes::{DiffLine, FileDiff, LineKind};

/// What a file's diff was read for: the lane's tree, base and head, and
/// the ticket's `updated_ms` (the head may be a branch name, which never
/// moves).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffRange {
    /// The lane's tree, where git runs.
    pub dir: PathBuf,
    /// The lane's base, the left side of `base...head`.
    pub base: String,
    /// The lane's head: a commit, or a branch name.
    pub head: String,
    /// The ticket's `updated_ms` when the range was taken, so a moved
    /// branch under an unchanged name still asks again.
    pub updated_ms: u64,
}

/// One file's diff: the last answer and the range it answered, kept
/// while a re-read is out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffRead {
    /// A read is on its way. Independent of `last`: a held answer stays
    /// while a re-read is out.
    pub in_flight: bool,
    /// The last answer and the range it was read for, if any came.
    pub last: Option<(DiffRange, Result<FileDiff, String>)>,
}

/// A stretch of a hunk's lines as the page draws it. Each range indexes
/// into the hunk's `lines`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Run {
    /// Lines drawn as they are.
    Lines(Range<usize>),
    /// Unchanged lines drawn as one row until the owner expands them.
    Folded(Range<usize>),
}

/// Unchanged lines kept beside a change when the rest of a run folds.
const FOLD_CONTEXT: usize = 3;

/// Fewer hidden lines than this are shown rather than folded: a fold
/// row in their place would save almost nothing.
const FOLD_MIN: usize = 4;

/// A hunk's lines as runs: unchanged stretches far from any change
/// fold, keeping `FOLD_CONTEXT` lines beside each change.
#[must_use]
pub fn runs(lines: &[DiffLine]) -> Vec<Run> {
    let changed: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind != LineKind::Context)
        .map(|(i, _)| i)
        .collect();
    let (Some(&first), Some(&last)) = (changed.first(), changed.last()) else {
        return if lines.is_empty() {
            Vec::new()
        } else {
            vec![Run::Lines(0..lines.len())]
        };
    };
    // The stretches to fold, in order.
    let mut folds = Vec::new();
    if first > FOLD_CONTEXT {
        folds.push(0..first - FOLD_CONTEXT);
    }
    for pair in changed.windows(2) {
        let (a, b) = (pair[0] + 1, pair[1]);
        if b > a + 2 * FOLD_CONTEXT {
            folds.push(a + FOLD_CONTEXT..b - FOLD_CONTEXT);
        }
    }
    if lines.len() > last + 1 + FOLD_CONTEXT {
        folds.push(last + 1 + FOLD_CONTEXT..lines.len());
    }
    let mut out = Vec::new();
    let mut at = 0;
    for f in folds.into_iter().filter(|f| f.len() >= FOLD_MIN) {
        if f.start > at {
            out.push(Run::Lines(at..f.start));
        }
        at = f.end;
        out.push(Run::Folded(f));
    }
    if at < lines.len() {
        out.push(Run::Lines(at..lines.len()));
    }
    out
}

type DiffKey = (String, String, String);

fn key(ticket: &str, lane: &str, path: &str) -> DiffKey {
    (ticket.to_owned(), lane.to_owned(), path.to_owned())
}

impl AppCore {
    /// The two diff actions, routed from `dispatch`.
    pub(super) fn diff_action(&mut self, action: AppAction, out: &mut Out) {
        match action {
            AppAction::ReadFileDiff {
                ticket,
                lane,
                path,
                old_path,
                range,
            } => self.read_file_diff(ticket, lane, path, old_path, range, out),
            AppAction::FileDiffRead {
                ticket,
                lane,
                path,
                range,
                result,
            } => self.file_diff_read(&ticket, &lane, &path, range, result),
            _ => {}
        }
    }

    /// Ask for a file's diff unless one is on its way or a good one is
    /// held for this range. A failure held for this range is asked again:
    /// that is the page's Retry.
    fn read_file_diff(
        &mut self,
        ticket: String,
        lane: String,
        path: String,
        old_path: Option<String>,
        range: DiffRange,
        out: &mut Out,
    ) {
        let read = self
            .dispatch
            .diffs
            .entry(key(&ticket, &lane, &path))
            .or_default();
        let held = matches!(&read.last, Some((r, Ok(_))) if *r == range);
        if read.in_flight || held {
            return;
        }
        read.in_flight = true;
        out.push(Effect::ReadFileDiff {
            ticket,
            lane,
            path,
            old_path,
            range,
        });
    }

    fn file_diff_read(
        &mut self,
        ticket: &str,
        lane: &str,
        path: &str,
        range: DiffRange,
        result: Result<FileDiff, String>,
    ) {
        let read = self
            .dispatch
            .diffs
            .entry(key(ticket, lane, path))
            .or_default();
        read.in_flight = false;
        read.last = Some((range, result));
    }

    /// A file's diff as held: the last answer, kept while a re-read is out.
    #[must_use]
    pub fn file_diff(&self, ticket: &str, lane: &str, path: &str) -> Option<&DiffRead> {
        self.dispatch.diffs.get(&key(ticket, lane, path))
    }

    /// Whether the page should ask for a file's diff: nothing is on its
    /// way and nothing is held for `range`. A failure at `range` is not
    /// due, so it is not asked again every frame.
    #[must_use]
    pub fn file_diff_due(&self, ticket: &str, lane: &str, path: &str, range: &DiffRange) -> bool {
        self.file_diff(ticket, lane, path).is_none_or(|read| {
            !read.in_flight && read.last.as_ref().is_none_or(|(r, _)| r != range)
        })
    }
}
