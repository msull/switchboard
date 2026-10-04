//! How a ticket went, from its record and the round files beside its
//! attempts: time per stage, the plan's size and review, the code
//! review's points, fix passes, rebases and the size of what reached
//! the PR. Pure: files are read through the caller's `read`, and git's
//! answer comes in as `range`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::git::RangeSize;
use crate::review::{
    LEFT_HEADING, open_points_of, point_text, reviewer_of, section_points, unqualified,
};
use crate::scheduler::REFRESH;
use crate::ticket::{AttemptKind, DecisionState, Ticket};

/// One pipeline stage's time: from its first attempt's start to its
/// last one's end (now, for one still open), and the time its
/// decisions waited on the user, shown apart.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StageTime {
    /// The stage's name.
    pub stage: String,
    /// Its attempts, reruns included.
    pub attempts: u32,
    /// First start to last end, the waiting included.
    pub ms: u64,
    /// The time its answered decisions waited on the user.
    pub waiting_ms: u64,
}

/// One round of a review: the points it raised that no earlier round
/// of the same work did (`None` when its file could not be read), and
/// the record's own open count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RoundReport {
    /// The round's number, from 1.
    pub n: u32,
    /// Points no earlier round raised; `None` when the file was unread.
    pub new: Option<u32>,
    /// Points open at the round, as the record counts them.
    pub open: u32,
}

/// One review attempt's rounds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReviewReport {
    /// The review's stage.
    pub stage: String,
    /// The attempt's number within it.
    pub attempt: u32,
    /// Its rounds, in order.
    pub rounds: Vec<RoundReport>,
}

/// How one ticket went, or (from `total`) several added up, for
/// `dispatch report`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct TicketReport {
    /// The ticket's id; empty in a total.
    pub id: String,
    /// The ticket's project; empty in a total.
    pub project: String,
    /// The source's title; empty in a total.
    pub title: String,
    /// The ticket's state, as `list` shows it; empty in a total.
    pub state: String,
    /// Each pipeline stage, in order.
    pub stages: Vec<StageTime>,
    /// The latest plan's length in lines, when there is one.
    pub plan_lines: Option<u32>,
    /// The latest plan's size in bytes, when there is one.
    pub plan_bytes: Option<u64>,
    /// Each plan review attempt, its rounds from the round files.
    pub plan_reviews: Vec<ReviewReport>,
    /// Plan review rounds across every attempt.
    pub plan_rounds: u32,
    /// Points the plan review's round files list.
    pub plan_points: u32,
    /// Each code review attempt.
    pub code_reviews: Vec<ReviewReport>,
    /// Code review rounds across every attempt.
    pub code_rounds: u32,
    /// Distinct points: a point carried open through later rounds, or
    /// into a rerun, counts once.
    pub code_points: u32,
    /// `code_points` by the reviewer that raised each.
    pub code_points_by_reviewer: BTreeMap<String, u32>,
    /// A round's file could not be read, so `code_points` may be short.
    pub code_incomplete: bool,
    /// Code review rounds an implementer worked on.
    pub fix_passes: u32,
    /// Rebaser runs, plus each lane's last bring-up that no rebaser
    /// did: a floor, since a lane keeps only its last bring-up.
    pub rebases: u32,
    /// The last pull request an attempt bound to.
    pub pr_url: Option<String>,
    /// Commits at the PR: git's count, else the last rewrite's.
    pub commits: Option<u32>,
    /// Git's answer for the PR's range, when it had one.
    pub range: Option<RangeSize>,
    /// How many tickets a total adds up; 1 for a ticket.
    pub tickets: u32,
}

/// The plan review's round files beside `subject`: `<stem>.feedback-<n>.md`.
/// Mirrors Switchboard's `core::workflow::round_paths`, which names the
/// files the review writes (Dispatch does not link the app): a rename
/// there is carried over here.
#[must_use]
pub fn round_file(subject: &Path, n: u32) -> PathBuf {
    let stem = subject
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    subject.with_file_name(format!("{stem}.feedback-{n}.md"))
}

/// The pull request a report sizes: the last attempt with one, its
/// lane (the attempt's context, else the only lane), the lane's base
/// and the PR's head.
#[must_use]
pub fn pr_range(t: &Ticket) -> Option<(String, String, String)> {
    let a = t.attempts.iter().rev().find(|a| a.pr.is_some())?;
    let pr = a.pr.as_ref()?;
    let lane = t
        .lanes
        .iter()
        .find(|l| l.name == a.context)
        .or_else(|| (t.lanes.len() == 1).then(|| &t.lanes[0]))?;
    Some((lane.name.clone(), lane.base_sha.clone()?, pr.head.clone()))
}

/// The report of one ticket. `stages` are its pipeline's stage names in
/// order; `read` reads a round file or the plan; `range` is git's size
/// of the PR's range when it could be read; `now_ms` ends a stage still
/// open.
#[must_use]
pub fn of(
    t: &Ticket,
    stages: &[String],
    read: &dyn Fn(&Path) -> Option<String>,
    range: Option<RangeSize>,
    now_ms: u64,
) -> TicketReport {
    let mut r = TicketReport {
        id: t.id.clone(),
        project: t.project.clone(),
        title: t.source.title.clone(),
        state: t.state.label(),
        tickets: 1,
        ..TicketReport::default()
    };
    for name in stages {
        let mine: Vec<_> = t.attempts_of(name).collect();
        let start = mine.iter().map(|a| a.started_ms).min();
        let end = mine.iter().map(|a| a.ended_ms.unwrap_or(now_ms)).max();
        let waiting_ms = t
            .decisions
            .iter()
            .filter(|d| &d.stage == name)
            .filter_map(|d| match &d.state {
                DecisionState::Answered { at_ms, .. } => Some(at_ms.saturating_sub(d.made_ms)),
                _ => None,
            })
            .sum();
        r.stages.push(StageTime {
            stage: name.clone(),
            attempts: u32::try_from(mine.len()).unwrap_or(u32::MAX),
            ms: start.zip(end).map_or(0, |(s, e)| e.saturating_sub(s)),
            waiting_ms,
        });
    }
    if let Some(text) = t.input("plan").and_then(|p| read(p)) {
        r.plan_lines = Some(u32::try_from(text.lines().count()).unwrap_or(u32::MAX));
        r.plan_bytes = Some(text.len() as u64);
    }
    for a in t
        .attempts
        .iter()
        .filter(|a| a.kind == AttemptKind::Workflow)
    {
        let Some(subject) = a.artifacts.values().next() else {
            continue;
        };
        let mut rounds = Vec::new();
        for n in 1.. {
            let Some(text) = read(&round_file(subject, n)) else {
                break;
            };
            let points = text
                .lines()
                .filter(|l| !l.starts_with(char::is_whitespace) && point_text(l).is_some())
                .count();
            let points = u32::try_from(points).unwrap_or(u32::MAX);
            rounds.push(RoundReport {
                n,
                new: Some(points),
                open: points,
            });
            r.plan_points += points;
        }
        r.plan_rounds += u32::try_from(rounds.len()).unwrap_or(u32::MAX);
        r.plan_reviews.push(ReviewReport {
            stage: a.stage.clone(),
            attempt: a.n,
            rounds,
        });
    }
    code_review(t, read, &mut r);
    r.rebases = rebases(t);
    r.pr_url = t
        .attempts
        .iter()
        .rev()
        .find_map(|a| a.pr.as_ref().map(|pr| pr.url.clone()));
    r.range = range;
    r.commits = range.map(|g| g.commits).or_else(|| {
        t.attempts.iter().rev().find_map(|a| {
            a.rewrite
                .as_ref()
                .filter(|w| w.after.is_some())
                .map(|w| w.to)
        })
    });
    r
}

/// Rebaser runs, plus each lane's last bring-up when no rebaser of that
/// lane ended before it: a bring-up after a rebaser only records the
/// rebaser's work, already counted. A lane keeps only its last bring-up,
/// so earlier clean ones are not counted and this is a floor.
fn rebases(t: &Ticket) -> u32 {
    let rebasers = t.attempts_of(REFRESH).count();
    let clean = t
        .lanes
        .iter()
        .filter_map(|l| Some((l, l.refreshed.as_ref()?)))
        .filter(|(l, r)| {
            !t.attempts_of(REFRESH)
                .any(|a| a.context == l.name && a.ended_ms.is_some_and(|e| e <= r.at_ms))
        })
        .count();
    u32::try_from(rebasers + clean).unwrap_or(u32::MAX)
}

/// Code review points counted by id, never by summing `open_points`,
/// which counts a point again in every round it stays open.
fn code_review(t: &Ticket, read: &dyn Fn(&Path) -> Option<String>, r: &mut TicketReport) {
    // The ids each attempt has seen, so a rerun that carries from it
    // starts from them.
    let mut seen_by: BTreeMap<(String, u32), BTreeSet<String>> = BTreeMap::new();
    for a in t.attempts.iter().filter(|a| a.kind == AttemptKind::Review) {
        let mut seen = a
            .carried_from
            .as_ref()
            .and_then(|from| seen_by.get(from).cloned())
            .unwrap_or_default();
        let mut rounds = Vec::new();
        for round in &a.rounds {
            if round.implementer.is_some() {
                r.fix_passes += 1;
            }
            let text = round.feedback.as_deref().and_then(read);
            let new = text.map(|text| {
                let mut new = 0u32;
                let listed = open_points_of(&text)
                    .into_iter()
                    .chain(section_points(&text, LEFT_HEADING));
                for (id, _) in listed {
                    let id = unqualified(&id).unwrap_or(&id).to_owned();
                    if seen.insert(id.clone()) {
                        new += 1;
                        *r.code_points_by_reviewer
                            .entry(reviewer_of(&id).to_owned())
                            .or_default() += 1;
                    }
                }
                new
            });
            if new.is_none() {
                r.code_incomplete = true;
            }
            r.code_points += new.unwrap_or(0);
            rounds.push(RoundReport {
                n: round.n,
                new,
                open: round.open_points,
            });
        }
        r.code_rounds += u32::try_from(rounds.len()).unwrap_or(u32::MAX);
        r.code_reviews.push(ReviewReport {
            stage: a.stage.clone(),
            attempt: a.n,
            rounds,
        });
        seen_by.insert((a.stage.clone(), a.n), seen);
    }
}

/// Every ticket's numbers added up, as one row.
#[must_use]
pub fn total(reports: &[TicketReport]) -> TicketReport {
    let mut sum = TicketReport::default();
    let mut stages: BTreeMap<String, StageTime> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for r in reports {
        sum.tickets += r.tickets;
        for s in &r.stages {
            if !order.contains(&s.stage) {
                order.push(s.stage.clone());
            }
            let e = stages.entry(s.stage.clone()).or_insert_with(|| StageTime {
                stage: s.stage.clone(),
                ..StageTime::default()
            });
            e.attempts += s.attempts;
            e.ms += s.ms;
            e.waiting_ms += s.waiting_ms;
        }
        sum.plan_lines = add(sum.plan_lines, r.plan_lines);
        sum.plan_bytes = add(sum.plan_bytes, r.plan_bytes);
        sum.plan_rounds += r.plan_rounds;
        sum.plan_points += r.plan_points;
        sum.code_rounds += r.code_rounds;
        sum.code_points += r.code_points;
        for (who, n) in &r.code_points_by_reviewer {
            *sum.code_points_by_reviewer.entry(who.clone()).or_default() += n;
        }
        sum.code_incomplete |= r.code_incomplete;
        sum.fix_passes += r.fix_passes;
        sum.rebases += r.rebases;
        sum.commits = add(sum.commits, r.commits);
        if let Some(g) = r.range {
            let s = sum.range.get_or_insert_with(RangeSize::default);
            s.commits += g.commits;
            s.files += g.files;
            s.insertions += g.insertions;
            s.deletions += g.deletions;
        }
    }
    sum.stages = order
        .into_iter()
        .filter_map(|name| stages.remove(&name))
        .collect();
    sum
}

fn add<T: std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a + b),
        (a, None) => a,
        (None, b) => b,
    }
}

/// `YYYY-MM-DD` as milliseconds at its UTC midnight.
#[must_use]
pub fn parse_date(text: &str) -> Option<u64> {
    let mut parts = text.trim().splitn(3, '-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: u32 = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    u64::try_from(days).ok().map(|days| days * 86_400_000)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard
/// Hinnant's algorithm).
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::scheduler::new_attempt;
    use crate::ticket::{
        Attempt, AttemptState, LaneRecord, PullRequestRecord, ReviewRound, ReviewerResult,
        ReviewerRun, Rewrite, RoundState,
    };

    fn files(map: &[(&str, &str)]) -> impl Fn(&Path) -> Option<String> {
        let map: BTreeMap<PathBuf, String> = map
            .iter()
            .map(|(p, t)| (PathBuf::from(p), (*t).to_owned()))
            .collect();
        move |p: &Path| map.get(p).cloned()
    }

    fn attempt(stage: &str, n: u32, kind: AttemptKind, started: u64, ended: u64) -> Attempt {
        Attempt {
            ended_ms: Some(ended),
            ..new_attempt(
                stage,
                n,
                "backend",
                kind,
                AttemptState::Complete,
                BTreeMap::new(),
                started,
            )
        }
    }

    fn round(n: u32, state: RoundState, open: u32, feedback: &str, fixed: bool) -> ReviewRound {
        ReviewRound {
            n,
            base: "base0000".into(),
            head: format!("head000{n}"),
            reviewers: ["correctness", "style"]
                .iter()
                .map(|name| ReviewerRun {
                    name: (*name).into(),
                    kind: "claude".into(),
                    dir: "/d".into(),
                    feedback: "/d/f.md".into(),
                    session: None,
                    launched: true,
                    stop_at_ms: None,
                    polls_since_stop: 0,
                    settle: None,
                    result: Some(ReviewerResult::Findings),
                })
                .collect(),
            state,
            feedback: Some(feedback.into()),
            open_points: open,
            fix_authorised: fixed,
            implementer: fixed.then(|| "impl".to_owned()),
            response: None,
            head_after: None,
            stop_at_ms: None,
            polls_since_stop: 0,
            settle: None,
            dirty_polls: 0,
            dirty_since_ms: None,
            started_ms: 0,
            ended_ms: None,
        }
    }

    fn lane() -> LaneRecord {
        LaneRecord {
            name: "backend".into(),
            worktree: "/wt".into(),
            branch: "dispatch/34-x".into(),
            project: None,
            chosen: true,
            setup_done: true,
            base_sha: Some("base0000".into()),
            refreshed: None,
            pushed: None,
            removed: false,
        }
    }

    fn stages() -> Vec<String> {
        ["plan", "review-plan", "implement", "review-code", "merge"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    /// A ticket shaped like a small closed one: one plan point over two
    /// rounds, one style point in code review, folded to one commit.
    fn closed_ticket() -> Ticket {
        let mut t = crate::ticket::blank();
        t.lanes.push(lane());
        let mut plan = attempt("plan", 1, AttemptKind::Agent, 0, 1_000);
        plan.artifacts
            .insert("plan".into(), "/t/plan/plan.md".into());
        let mut review_plan = attempt("review-plan", 1, AttemptKind::Workflow, 1_000, 5_000);
        review_plan
            .artifacts
            .insert("plan".into(), "/t/review-plan/plan.md".into());
        let implement = attempt("implement", 1, AttemptKind::Agent, 5_000, 9_000);
        let mut code = attempt("review-code", 1, AttemptKind::Review, 9_000, 20_000);
        code.rounds = vec![
            round(1, RoundState::Fixed, 1, "/t/rc/r1.md", true),
            round(2, RoundState::Converged, 0, "/t/rc/r2.md", false),
        ];
        code.rewrite = Some(Rewrite {
            mode: crate::history::Commits::Fold,
            before: "head0002".into(),
            after: Some("fold0001".into()),
            from: 2,
            to: 1,
            skipped: None,
            at_ms: 20_000,
        });
        let mut merge = attempt("merge", 1, AttemptKind::GateOnly, 20_000, 30_000);
        merge.pr = Some(PullRequestRecord {
            provider: "github".into(),
            repo: "o/r".into(),
            number: 35,
            url: "https://example.com/pull/35".into(),
            head: "fold0001".into(),
            checks: "merged".into(),
            checked_ms: 0,
            error_since_ms: None,
        });
        t.attempts = vec![plan, review_plan, implement, code, merge];
        t
    }

    /// A conflicting rebase is one rebaser run and the bring-up after
    /// it, counted once; a lane brought up with no rebaser counts too.
    #[test]
    fn a_rebase_and_the_bring_up_after_it_count_once() {
        let mut t = closed_ticket();
        let refreshed = |at_ms| crate::ticket::Refreshed {
            from: "base0000".into(),
            to: "base0001".into(),
            commits: true,
            notes: None,
            at_ms,
        };
        let mut rebased = lane();
        rebased.refreshed = Some(refreshed(12_000));
        let mut clean = lane();
        clean.name = "frontend".into();
        clean.refreshed = Some(refreshed(15_000));
        t.lanes = vec![rebased, clean];
        t.attempts
            .push(attempt(REFRESH, 1, AttemptKind::Agent, 10_000, 11_000));
        let r = of(&t, &stages(), &files(&[]), None, 99_000);
        assert_eq!(r.rebases, 2);
    }

    const PLAN_ROUND_1: &str = "# Feedback\n\n1. Name the lock order.\n   - an indented aside\n";
    const CODE_ROUND_1: &str = "## Open\n\n- r1/style-1 (style): rename tmp\n";
    const CODE_ROUND_2: &str = "## Left to the merge\n\n- r1/style-1 (style): rename tmp\n";

    #[test]
    fn a_closed_ticket_counts_plan_and_code_points_once_each() {
        let t = closed_ticket();
        let read = files(&[
            // The plan a later stage reads is the reviewed copy.
            ("/t/review-plan/plan.md", "# plan\nsteps\nmore\n"),
            ("/t/review-plan/plan.feedback-1.md", PLAN_ROUND_1),
            (
                "/t/review-plan/plan.feedback-2.md",
                "No further feedback.\n",
            ),
            ("/t/rc/r1.md", CODE_ROUND_1),
            ("/t/rc/r2.md", CODE_ROUND_2),
        ]);
        let range = RangeSize {
            commits: 1,
            files: 3,
            insertions: 40,
            deletions: 2,
        };
        let r = of(&t, &stages(), &read, Some(range), 99_000);
        assert_eq!((r.plan_lines, r.plan_bytes), (Some(3), Some(18)));
        assert_eq!((r.plan_rounds, r.plan_points), (2, 1));
        assert_eq!((r.code_rounds, r.code_points), (2, 1));
        assert_eq!(
            r.code_points_by_reviewer,
            BTreeMap::from([("style".into(), 1)])
        );
        assert!(!r.code_incomplete);
        assert_eq!(r.fix_passes, 1);
        assert_eq!(r.commits, Some(1));
        assert_eq!(r.pr_url.as_deref(), Some("https://example.com/pull/35"));
        assert_eq!(r.stages[3].ms, 11_000);
        assert_eq!(
            pr_range(&t),
            Some(("backend".into(), "base0000".into(), "fold0001".into()))
        );

        let r = of(&t, &stages(), &read, None, 99_000);
        assert_eq!(r.commits, Some(1), "the rewrite's count when git has none");
        assert_eq!(r.range, None);
    }

    /// A point carried open through a later round, and into a rerun
    /// that continues the attempt, is still one point.
    #[test]
    fn a_carried_point_counts_once() {
        let mut t = crate::ticket::blank();
        t.lanes.push(lane());
        let mut first = attempt("review-code", 1, AttemptKind::Review, 0, 10);
        first.rounds = vec![
            round(1, RoundState::Fixed, 2, "/r1.md", true),
            round(2, RoundState::Fixed, 1, "/r2.md", true),
            round(3, RoundState::Converged, 0, "/r3.md", false),
        ];
        let mut rerun = attempt("review-code", 2, AttemptKind::Review, 20, 30);
        rerun.carried_from = Some(("review-code".into(), 1));
        rerun.rounds = vec![round(1, RoundState::Fixed, 1, "/a2r1.md", true)];
        t.attempts = vec![first, rerun];
        let read = files(&[
            (
                "/r1.md",
                "## Open\n\n- r1/style-1 (style): rename\n- r1/correctness-1 (correctness): race\n",
            ),
            ("/r2.md", "## Open\n\n- r1/style-1 (style): rename\n"),
            ("/r3.md", "No open points.\n"),
            ("/a2r1.md", "## Open\n\n- a1/r1/style-1 (style): rename\n"),
        ]);
        let r = of(&t, &stages(), &read, None, 50);
        assert_eq!(r.code_points, 2);
        assert_eq!(
            r.code_points_by_reviewer,
            BTreeMap::from([("correctness".into(), 1), ("style".into(), 1)])
        );
        let rounds = &r.code_reviews[0].rounds;
        assert_eq!((rounds[1].new, rounds[1].open), (Some(0), 1));
        assert_eq!(r.code_reviews[1].rounds[0].new, Some(0));
        let total = total(&[r.clone(), r]);
        assert_eq!((total.tickets, total.code_points), (2, 4));
    }

    #[test]
    fn a_round_without_its_file_marks_the_count_incomplete() {
        let mut t = crate::ticket::blank();
        let mut a = attempt("review-code", 1, AttemptKind::Review, 0, 10);
        a.rounds = vec![round(1, RoundState::Fixed, 1, "/gone.md", true)];
        t.attempts = vec![a];
        let r = of(&t, &stages(), &files(&[]), None, 50);
        assert!(r.code_incomplete);
        assert_eq!(r.code_reviews[0].rounds[0].new, None);
    }

    #[test]
    fn a_date_is_its_utc_midnight() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        assert_eq!(parse_date("2026-10-04"), Some(1_791_072_000_000));
        assert_eq!(parse_date("2000-03-01"), Some(951_868_800_000));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("yesterday"), None);
    }
}
