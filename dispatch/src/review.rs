//! The code review stage: several reviewers read a lane's branch at
//! once, their findings are gathered into one numbered file, a fresh
//! implementer addresses them on the branch, the stage's checks run at
//! the new head, and the next round reads that; until no point is open
//! or the cap is reached. Every head a round read, every reviewer's
//! completion and every check's exit is on the attempt's round record,
//! bound to the head it was made at.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use switchboard_control::{self as wire, Body, Reply};

use crate::history::{self, Commits};
use crate::pipeline::{Gate, OperatorKind, Pipeline, Stage};
use crate::scheduler::{
    Ask, NO_SUCH_SESSION, Runner, SocketDown, asks_again, busy, checks_env, env_for, find_attempt,
    find_attempt_mut, guidance_prelude, held_in, idle_polls, lane_gate_argv, latest_attempt,
    may_rerun, new_attempt, next_n, primary_tree, record_of, rework_key, sent_back, session_kind,
    settle_file, vars_for,
};
use crate::template::Vars;
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, DIRTY_POLLS, DecisionKind, GateRun, KEPT_BY_HAND,
    PUBLISHED, ProjectState, Refreshed, ReviewRound, ReviewerResult, ReviewerRun, Rewrite,
    RoundState, STOP_IDLE_POLLS, Ticket,
};

/// What a reviewer writes when it has nothing to report, unless the
/// stage says otherwise.
pub const NO_FINDINGS: &str = "No findings.";

/// The reviewer prompt a stage gets when it gives none.
const REVIEW_PROMPT: &str = "Review branch {branch} in {worktree}: the commits {base}..{head} (see git diff {base} {head}). Write your findings to {feedback} as a Markdown list, one point per line starting with \"- \", each naming the file and saying why it matters. If you find nothing, write exactly this line alone: {no_feedback}. Change nothing in {worktree}.";

/// The addition when earlier rounds left points open.
const REVIEW_CARRY: &str = "Points still open from earlier rounds are listed at {previous_feedback} with the implementer's answers at {previous_response}. For each open point write a line \"withdraw <id>\" if the answer satisfies you, or \"keep <id>: why\" if it does not.";

/// The addition to round 1 of an attempt that continues an earlier one.
const REVIEW_CARRIED: &str = "An earlier attempt of this review (attempt {carried_n}) reviewed this branch at {old_head} through round {through}. These points were settled there; do not raise them again:\n{settled}\nThese are still open:\n{open}\n{scope} For each open point write a line \"withdraw <id>\" if the change settles it, or \"keep <id>: why\" if it does not.";

/// The mark on a carried open point whose fix no round has read.
const UNCHECKED_FIX: &str = "(answered fixed; check the fix)";

/// What a carried round reads when the branch moved since.
const REVIEW_CARRIED_RANGE: &str =
    "Review the change from {old_head} to {head} ({range}) and the open points only.";

/// What a carried round reads when the branch did not move since.
const REVIEW_CARRIED_STILL: &str = "The branch has not moved since; review the open points only.";

/// The addition to the first review after the branch was rebased.
const REVIEW_REBASED: &str = "The base moved from {from} to {to} (git log {from}..{to}) and the branch was rebased onto it. Check explicitly that both sides of every conflicted hunk are present and that the base's additions in {from}..{to} are unchanged by the branch.";

/// The rebase check when the base the branch moved from was not recorded.
const REVIEW_REBASED_UNKNOWN: &str = "The branch was rebased onto {to} from a base that was not recorded. Check explicitly that both sides of every conflicted hunk are present and that the base's additions the rebase brought in are unchanged by the branch.";

/// The addition when the plan has a decisions section.
const REVIEW_DECIDED: &str = "The plan at {plan} settled these decisions:\n\n{decisions}\n\nA point that contests one of them is out of scope for this review: write it as \"- decided: <the decision>: why\" and it is listed as found but not done.";

/// The addition every agent reviewer gets.
const REVIEW_STYLE: &str = "Start a point that is only about wording, naming, comments or layout with \"style: \". Style points do not hold the review open after its early rounds.";

/// The reviewer whose untagged points are style points.
const STYLE_REVIEWER: &str = "style";

/// The round file's section of style points a converged round leaves.
const LEFT_HEADING: &str = "Left to the merge";

/// The round file's section of points that contest the plan.
const DECIDED_HEADING: &str = "Found but not done";

/// The round file's section of what reviewers said of the round that
/// is not a point.
const NOTES_HEADING: &str = "Reviewer notes";

/// The implementer prompt a stage gets when it gives none.
const FIX_PROMPT: &str = "Reviewers of branch {branch} in {worktree} left points at {feedback}. Address each one on the branch: fix it, or dispute it with your reasons. Commit so the tree is clean. Then write {response}, answering every point by its id, one line each: \"- <id>: fixed <what>\" or \"- <id>: disputed <why>\".";

/// A reviewer's or an implementer's session view, or why there is none
/// this pass.
enum Seen {
    View(Box<wire::SessionView>),
    Gone,
    Unknown,
}

impl Runner {
    /// One pass over a code review stage: every context's attempt
    /// polled or started, the ticket advanced when all are complete.
    pub(crate) fn review_stage(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<()> {
        let Some(contexts) = self.contexts_or_park(t, ps, p, stage, now_ms)? else {
            return Ok(());
        };
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let last = latest_attempt(t, &stage.name, &ctx).cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    self.poll_review(t, ps, p, stage, &a, &cwd, lane.as_deref(), now_ms)?;
                }
                Some(a) => {
                    all_complete = false;
                    let held = held_in(t, &stage.name, &ctx);
                    let sent_back = sent_back(t, stage, &ctx);
                    if !held && (may_rerun(t, &a) || sent_back) {
                        self.start_review(t, ps, p, stage, &ctx, &cwd, lane.as_deref(), now_ms)?;
                    } else if asks_again(t, &a, sent_back) {
                        self.ask_rerun(t, ps, &a, now_ms)?;
                    }
                }
                None => {
                    all_complete = false;
                    if !held_in(t, &stage.name, &ctx) {
                        self.start_review(t, ps, p, stage, &ctx, &cwd, lane.as_deref(), now_ms)?;
                    }
                }
            }
            if !t.active() {
                return Ok(());
            }
        }
        if all_complete {
            self.advance(t, now_ms)?;
        }
        Ok(())
    }

    /// A new attempt of the stage in a context, and its first round.
    #[allow(clippy::too_many_arguments)]
    fn start_review(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        ctx: &str,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let project = match self.ensure_project(t, ps, p, now_ms) {
            Ok(id) => id,
            Err(e) if e.is::<SocketDown>() => return Err(e),
            Err(e) => {
                return self.park(t, ps, &format!("stage {}: {e:#}", stage.name), now_ms);
            }
        };
        if !self.ensure_setup(t, ps, p, cwd, now_ms)? {
            return Ok(());
        }
        let n = next_n(t, &stage.name);
        self.attempt_dir(t, &stage.name, n, ctx)?;
        let mut attempt = new_attempt(
            &stage.name,
            n,
            ctx,
            AttemptKind::Review,
            AttemptState::Running,
            std::collections::BTreeMap::new(),
            now_ms,
        );
        attempt.project = Some(project);
        // A note from a later human gate goes to the first implementer
        // of this attempt, not to the reviewers. It leaves the ticket
        // now, so it does not keep the context sent back, and a note a
        // fix pass has used is not given to the next attempt.
        attempt.rework = t
            .rework
            .remove(&rework_key(&stage.name, ctx))
            .or_else(|| unspent_note(t, &stage.name, ctx, n));
        if !attempt.rework.as_deref().is_some_and(starts_over) {
            attempt.carried_from = carry_source(t, &stage.name, ctx, n);
        }
        t.attempts.push(attempt);
        self.save_ticket(t, now_ms)?;
        self.open_round(t, ps, p, stage, &(stage.name.clone(), n), cwd, lane, now_ms)
    }

    /// The base a lane's review diffs against: resolved once at the
    /// cut and kept on the lane; a lane cut before that was recorded
    /// gets it from the merge base now, and keeps it.
    fn base_of(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<String> {
        let Some(i) = lane.and_then(|l| t.lanes.iter().position(|x| x.name == l)) else {
            return self.git.merge_base(
                cwd,
                &format!("{}/{}", p.project.remote, p.project.base),
                "HEAD",
            );
        };
        if let Some(sha) = &t.lanes[i].base_sha {
            return Ok(sha.clone());
        }
        let def = p.lane(&t.lanes[i].name);
        let upstream = def.map_or_else(
            || format!("{}/{}", p.project.remote, p.project.base),
            |l| format!("{}/{}", p.lane_remote(l), p.lane_base(l)),
        );
        let sha = self.git.merge_base(cwd, &upstream, "HEAD")?;
        t.lanes[i].base_sha = Some(sha.clone());
        self.save_ticket(t, now_ms)?;
        Ok(sha)
    }

    /// A round opened: the tree clean at a head that is where the last
    /// round left it, the base and head on the record, every reviewer
    /// started.
    #[allow(clippy::too_many_arguments)]
    fn open_round(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        key: &(String, u32),
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let (stage_name, n) = key.clone();
        if !self.git.is_clean(cwd)? {
            let reason = format!(
                "the tree at {} is not clean when a review round would start",
                cwd.display()
            );
            return self.fail_attempt(t, ps, &stage_name, n, &reason, now_ms);
        }
        let head = self.git.head(cwd)?;
        let base = self.base_of(t, p, cwd, lane, now_ms)?;
        let a = record_of(t, &key.0, key.1).clone();
        if let Some(prev) = a.rounds.last() {
            let expected = prev.head_after.clone().unwrap_or_else(|| prev.head.clone());
            if expected != head {
                return self.park(
                    t,
                    ps,
                    &format!(
                        "stage {stage_name} ({}): the branch moved from {expected} to {head} by something other than the implementer; the review's evidence is history",
                        a.context
                    ),
                    now_ms,
                );
            }
        }
        let round_n = u32::try_from(a.rounds.len()).unwrap_or(u32::MAX) + 1;
        let rdir = self
            .attempt_dir(t, &stage_name, n, &a.context)?
            .join(format!("r{round_n}"));
        let mut reviewers = Vec::new();
        for name in &stage.reviewers {
            let op = &p.operators[name];
            let dir = rdir.join(name);
            std::fs::create_dir_all(&dir)?;
            let feedback = dir.join("feedback.md");
            reviewers.push(ReviewerRun {
                name: name.clone(),
                kind: match op.kind {
                    OperatorKind::Claude => "claude",
                    OperatorKind::Codex => "codex",
                    OperatorKind::Command => "command",
                }
                .into(),
                dir,
                feedback,
                session: None,
                launched: false,
                stop_at_ms: None,
                polls_since_stop: 0,
                settle: None,
                result: None,
            });
        }
        log::info!(
            "ticket {} {stage_name}/{} review round {round_n} at {head} over {base}",
            t.id,
            a.context
        );
        let attempt = record_of(t, &key.0, key.1);
        attempt.gate = None;
        for r in &reviewers {
            attempt
                .artifacts
                .insert(format!("r{round_n}/{}", r.name), r.feedback.clone());
        }
        attempt.rounds.push(ReviewRound {
            n: round_n,
            base,
            head,
            reviewers,
            state: RoundState::Reviewing,
            feedback: None,
            open_points: 0,
            fix_authorised: false,
            implementer: None,
            response: None,
            head_after: None,
            stop_at_ms: None,
            polls_since_stop: 0,
            settle: None,
            dirty_polls: 0,
            started_ms: now_ms,
            ended_ms: None,
        });
        self.save_ticket(t, now_ms)?;
        for name in stage.reviewers.clone() {
            self.launch_reviewer(t, ps, p, stage, key, round_n, &name, cwd, lane, now_ms)?;
            if !t.active() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// One reviewer started: an agent in a session (Claude Code in the
    /// tree with the reviewer directory writable, Codex in that
    /// directory), or a command as a child of the runner with its
    /// intent recorded first.
    #[allow(clippy::too_many_arguments)]
    fn launch_reviewer(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        key: &(String, u32),
        round_n: u32,
        name: &str,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let op = p.operators[name].clone();
        let a = record_of(t, &key.0, key.1).clone();
        let Some(round) = a.rounds.iter().find(|r| r.n == round_n).cloned() else {
            return Ok(());
        };
        let Some(r) = round.reviewers.iter().find(|r| r.name == name).cloned() else {
            return Ok(());
        };
        if r.session.is_some() || r.launched || r.result.is_some() {
            return Ok(());
        }
        if op.kind == OperatorKind::Command {
            let dir = if op.run_in == "root" {
                primary_tree(t, p).unwrap_or_else(|| cwd.to_path_buf())
            } else {
                cwd.to_path_buf()
            };
            let mut env = env_for(
                t,
                lane,
                lane.and_then(|l| t.lanes.iter().find(|x| x.name == l))
                    .map(|l| l.branch.as_str()),
            );
            env.push(("DISPATCH_BASE".to_owned(), round.base.clone()));
            env.push(("DISPATCH_HEAD".to_owned(), round.head.clone()));
            env.push(("DISPATCH_TREE".to_owned(), cwd.display().to_string()));
            // The intent is on disk before anything runs: a restart
            // that finds it with no result fails the reviewer rather
            // than starting a second copy.
            reviewer_mut(t, key, round_n, name).launched = true;
            self.save_ticket(t, now_ms)?;
            let check_key = reviewer_key(t, key, round_n, name);
            let stderr = r.dir.join("stderr");
            if let Err(e) =
                self.git
                    .start_reviewer(&check_key, &dir, &op.argv, &env, &r.feedback, &stderr)
            {
                reviewer_mut(t, key, round_n, name).result = Some(ReviewerResult::Failed {
                    reason: format!("could not start: {e:#}"),
                });
            }
            return self.save_ticket(t, now_ms);
        }
        let project = a.project.clone().unwrap_or_default();
        let prompt = Self::reviewer_prompt(t, p, stage, &a, &round, &r, cwd, lane);
        let mut args = op.args.clone();
        let session_cwd = if op.kind.reviews_in_tree() {
            args.extend(op.kind.write_flags(&r.dir));
            cwd.to_path_buf()
        } else {
            r.dir.clone()
        };
        let launch = if args.is_empty() {
            wire::Launch::Shell
        } else {
            wire::Launch::Argv(args)
        };
        let notes = format!(
            "Dispatch ticket {} · #{} {} · stage {} attempt {} · round {round_n} reviewer {name}",
            t.id,
            t.source.number.unwrap_or(0),
            t.source.title,
            key.0,
            key.1
        );
        let reply = self.send(
            t,
            ps,
            Some(key.clone()),
            &format!("reviewer:{round_n}:{name}"),
            Body::SessionNew {
                project,
                name: name.to_owned(),
                session_kind: session_kind(op.kind),
                cwd: session_cwd,
                launch,
                prompt: Some(prompt),
                notes,
            },
            now_ms,
        )?;
        if let Reply::Failed { reason } = reply {
            reviewer_mut(t, key, round_n, name).result = Some(ReviewerResult::Failed {
                reason: format!("could not start: {reason}"),
            });
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// The reviewer's prompt: the operator's guidance, then the stage's
    /// review template (or the default), with the round's base and
    /// head, the tree, the plan and the earlier rounds' open points;
    /// then what a carried attempt settled and left open, the rebase to
    /// check, the plan's decisions and how to tag a style point.
    #[allow(clippy::too_many_arguments)]
    fn reviewer_prompt(
        t: &Ticket,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        round: &ReviewRound,
        r: &ReviewerRun,
        cwd: &Path,
        lane: Option<&str>,
    ) -> String {
        let mut vars = vars_for(t, p, lane);
        vars.set("base", round.base.clone())
            .set("head", round.head.clone())
            .set("worktree", cwd.display().to_string())
            .set("feedback", r.feedback.display().to_string())
            .set(
                "no_feedback",
                stage
                    .no_feedback
                    .clone()
                    .unwrap_or_else(|| NO_FINDINGS.to_owned()),
            );
        if let Some(plan) = t.input("plan") {
            vars.set("plan", plan.display().to_string());
        }
        let previous = a.rounds.iter().rev().find(|x| x.n < round.n);
        let carried = previous.is_some_and(|x| x.open_points > 0);
        if let Some(prev) = previous {
            vars.set(
                "previous_feedback",
                prev.feedback
                    .as_ref()
                    .map(|f| f.display().to_string())
                    .unwrap_or_default(),
            )
            .set(
                "previous_response",
                prev.response
                    .as_ref()
                    .map(|f| f.display().to_string())
                    .unwrap_or_default(),
            );
        }
        let mut prompt = guidance_prelude(&p.operators[&r.name].guidance, &vars);
        prompt.push_str(&vars.render(stage.review_prompt.as_deref().unwrap_or(REVIEW_PROMPT)));
        if round.n == 1
            && let Some(carry) = carry_of(t, a, &no_feedback_of(stage))
        {
            prompt.push_str("\n\n");
            prompt.push_str(&carried_text(&carry, round));
        }
        if carried {
            prompt.push_str("\n\n");
            prompt.push_str(&vars.render(REVIEW_CARRY));
        }
        if let Some(moved) = rebase_to_check(t, a, round, lane) {
            prompt.push_str("\n\n");
            prompt.push_str(&rebased_text(moved));
        }
        if let Some(plan) = t.input("plan")
            && let Ok(text) = std::fs::read_to_string(plan)
        {
            prompt.push_str("\n\n");
            match decisions_section(&text) {
                Some(decisions) => {
                    vars.set("decisions", decisions);
                    prompt.push_str(&vars.render(REVIEW_DECIDED));
                }
                None => {
                    let _ = write!(prompt, "The plan at {} lists no decisions.", plan.display());
                }
            }
        }
        prompt.push_str("\n\n");
        prompt.push_str(REVIEW_STYLE);
        prompt
    }

    /// An open attempt, by its last round's state.
    #[allow(clippy::too_many_arguments)]
    fn poll_review(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let Some(round) = a.rounds.last().cloned() else {
            // The attempt was written and the runner stopped before its
            // first round: open it now.
            return self.open_round(t, ps, p, stage, &key, cwd, lane, now_ms);
        };
        match round.state {
            RoundState::Reviewing => self.poll_reviewers(t, ps, p, stage, a, &round, cwd, now_ms),
            RoundState::Findings => {
                if round.fix_authorised {
                    self.start_fix(t, ps, p, stage, &key, &round, cwd, lane, now_ms)
                } else {
                    Ok(())
                }
            }
            RoundState::Fixing => self.poll_fix(t, ps, a, &round, cwd, now_ms),
            RoundState::Fixed | RoundState::Converged | RoundState::Accepted => {
                // A rewrite whose intent was saved and whose end was not:
                // the checks passed, and the branch may have moved since.
                if let Some(r) = a.rewrite.as_ref().filter(|r| r.after.is_none()) {
                    let before = r.before.clone();
                    self.resume_rewrite(t, ps, p, stage, a, &before, cwd, lane, now_ms)
                } else if a.gate.is_some() {
                    self.poll_checks(t, ps, p, stage, a, &round, cwd, lane, now_ms)
                } else {
                    self.start_checks(t, ps, p, stage, a, &round, cwd, lane, now_ms)
                }
            }
            RoundState::Failed { .. } => Ok(()),
        }
    }

    /// Every reviewer not finished looked at; when all are, the round
    /// is judged: a failed reviewer fails the attempt (its siblings
    /// retired first), a changed tree voids the round, and otherwise
    /// the findings are gathered and the round converges or asks.
    #[allow(clippy::too_many_arguments)]
    fn poll_reviewers(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        round: &ReviewRound,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        for r in &round.reviewers {
            if r.result.is_some() {
                continue;
            }
            if r.kind == "command" {
                self.poll_command_reviewer(t, &key, round.n, r);
            } else {
                self.poll_agent_reviewer(t, ps, &key, round.n, r, now_ms)?;
            }
        }
        self.save_ticket(t, now_ms)?;
        let round = record_of(t, &key.0, key.1)
            .rounds
            .iter()
            .find(|x| x.n == round.n)
            .cloned()
            .expect("the round exists");
        if !self.fail_round_if_a_reviewer_did(t, ps, &key, &round, now_ms)? {
            return Ok(());
        }
        if round.reviewers.iter().any(|r| r.result.is_none()) {
            return Ok(());
        }
        let head = self.git.head(cwd)?;
        if !self.git.is_clean(cwd)? || head != round.head {
            let reason = format!(
                "round {}: the tree at {} changed while the reviewers read it (head {} then {head}); the round's evidence is void",
                round.n,
                cwd.display(),
                round.head
            );
            return self.fail_round(t, ps, &key, round.n, &reason, now_ms);
        }
        let Aggregated {
            text,
            open,
            left,
            decided,
        } = self.judge(t, stage, a, &round);
        let path = round
            .reviewers
            .first()
            .and_then(|r| r.dir.parent())
            .map_or_else(|| cwd.join("feedback.md"), |d| d.join("feedback.md"));
        std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        let attempt = record_of(t, &key.0, key.1);
        attempt
            .artifacts
            .insert(format!("r{}/feedback", round.n), path.clone());
        let r = attempt
            .rounds
            .iter_mut()
            .find(|x| x.n == round.n)
            .expect("the round exists");
        r.feedback = Some(path.clone());
        r.open_points = open;
        if open == 0 {
            let mut tally = if left.is_empty() {
                String::new()
            } else {
                format!(", {} left to the merge", left.len())
            };
            if !decided.is_empty() {
                let _ = write!(tally, ", {} found but not done", decided.len());
            }
            log::info!(
                "ticket {} {}/{} round {} converged at {head}{tally}",
                t.id,
                key.0,
                a.context,
                round.n
            );
            set_round_state(t, &key, round.n, RoundState::Converged, now_ms);
            return self.save_ticket(t, now_ms);
        }
        set_round_state(t, &key, round.n, RoundState::Findings, now_ms);
        self.ask_about_findings(t, ps, p, stage, a, &round, open, &path, now_ms)
    }

    /// A round's findings gathered and judged: the points open coming
    /// into it (the previous round's, or a carried attempt's for its
    /// first round), and the live `style_rounds` against the round's
    /// number counted on from the carried attempt's rounds.
    fn judge(&self, t: &Ticket, stage: &Stage, a: &Attempt, round: &ReviewRound) -> Aggregated {
        let carry = carry_of(t, a, &no_feedback_of(stage));
        let earlier = if round.n > 1 {
            let previous = a.rounds.iter().rev().find(|x| x.n < round.n);
            carried_points(previous)
        } else {
            carry.as_ref().map(|c| c.open.clone()).unwrap_or_default()
        };
        let counted = round.n + carry.as_ref().map_or(0, |c| c.through);
        let style_rounds = self.live_style_rounds(t, stage);
        aggregate(t, stage, &a.context, round, &earlier, counted, style_rounds)
    }

    /// The stage's `style_rounds` from the project's live pipeline file
    /// (the pull-request one for a ticket taken from a pull request),
    /// so an edit takes effect at the next round; the ticket's copy
    /// when the file cannot be read or no longer has the stage.
    fn live_style_rounds(&self, t: &Ticket, stage: &Stage) -> u32 {
        let path = if t.source.pull_requests.is_empty() {
            self.data.pipeline(&t.project)
        } else {
            self.data.pr_pipeline(&t.project)
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| Pipeline::parse(&text).ok())
            .and_then(|p| p.stages.into_iter().find(|s| s.name == stage.name))
            .map_or_else(|| stage.style_rounds(), |s| s.style_rounds())
    }

    /// A failed reviewer fails the attempt, once its siblings are
    /// retired (true when nothing failed, or the failure is recorded).
    /// False while siblings are still being retired.
    fn fail_round_if_a_reviewer_did(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        key: &(String, u32),
        round: &ReviewRound,
        now_ms: u64,
    ) -> Result<bool> {
        let failed: Vec<String> = round
            .reviewers
            .iter()
            .filter_map(|r| match &r.result {
                Some(ReviewerResult::Failed { reason }) => Some(format!("{}: {reason}", r.name)),
                _ => None,
            })
            .collect();
        if !failed.is_empty() {
            // The siblings go first, so nothing of the round keeps
            // running once it reads as failed.
            let others: Vec<String> = round
                .reviewers
                .iter()
                .filter(|r| r.result.is_none())
                .filter_map(|r| r.session.clone())
                .collect();
            if !self.retire_processes(t, ps, &others, now_ms)? {
                return Ok(false);
            }
            let a = record_of(t, &key.0, key.1).clone();
            self.kill_review_commands(t, &a);
            set_round_state(
                t,
                key,
                round.n,
                RoundState::Failed {
                    reason: failed.join("; "),
                },
                now_ms,
            );
            let reason = format!("round {}: reviewer {}", round.n, failed.join("; reviewer "));
            self.fail_attempt(t, ps, &key.0, key.1, &reason, now_ms)?;
            return Ok(false);
        }
        Ok(true)
    }

    /// Findings at a head: at the cap, the cap question; otherwise the
    /// fix pass by the dial, or the round question.
    #[allow(clippy::too_many_arguments)]
    fn ask_about_findings(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        round: &ReviewRound,
        open: u32,
        path: &Path,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let head = round.head.clone();
        let passes = u32::try_from(a.rounds.len()).unwrap_or(u32::MAX);
        let cap = stage.review_cap() + u32::from(a.extra_pass);
        let short: String = head.chars().take(8).collect();
        if passes >= cap {
            let question = format!(
                "{} ({}): round {} of {cap} left {open} point(s) open at {short} (findings at {}). accept takes the reviewed head as it is; more is one fix pass, the checks, then one more review pass; park stops.",
                key.0,
                a.context,
                round.n,
                path.display()
            );
            return self.ensure_decision(
                t,
                ps,
                Ask {
                    stage: &key.0,
                    name: "review-cap",
                    kind: DecisionKind::Permission,
                    question,
                    options: &["accept", "more", "park"],
                    recommendation: None,
                    attempt: Some(key.clone()),
                },
                now_ms,
            );
        }
        if p.dial("review-code") == "auto" {
            record_of(t, &key.0, key.1)
                .rounds
                .iter_mut()
                .find(|x| x.n == round.n)
                .expect("the round exists")
                .fix_authorised = true;
            return self.save_ticket(t, now_ms);
        }
        let question = format!(
            "{} ({}): round {} found {open} point(s) at {short} (findings at {}). fix starts a fresh implementer on them; accept takes the head as it is; park stops.",
            key.0,
            a.context,
            round.n,
            path.display()
        );
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage: &key.0,
                name: "review-code",
                kind: DecisionKind::Permission,
                question,
                options: &["fix", "accept", "park"],
                recommendation: None,
                attempt: Some(key.clone()),
            },
            now_ms,
        )
    }

    /// A command reviewer's child: still running, exited, or lost with
    /// a runner that restarted (failed, never started again).
    fn poll_command_reviewer(
        &mut self,
        t: &mut Ticket,
        key: &(String, u32),
        round_n: u32,
        r: &ReviewerRun,
    ) {
        if !r.launched {
            return;
        }
        let check_key = reviewer_key(t, key, round_n, &r.name);
        let result = match self.git.poll_check(&check_key) {
            None => return,
            Some(Err(e)) => ReviewerResult::Failed {
                reason: format!("lost: {e:#}"),
            },
            Some(Ok(0)) => ReviewerResult::Clean,
            Some(Ok(1)) => {
                let out = std::fs::read_to_string(&r.feedback).unwrap_or_default();
                if out.trim().is_empty() {
                    ReviewerResult::Failed {
                        reason: "exited 1 with nothing on stdout".into(),
                    }
                } else {
                    ReviewerResult::Findings
                }
            }
            Some(Ok(code)) => ReviewerResult::Failed {
                reason: format!(
                    "exited {code}; stderr at {}",
                    r.dir.join("stderr").display()
                ),
            },
        };
        log::info!(
            "ticket {} {}/{} round {round_n} reviewer {}: {result:?}",
            t.id,
            key.0,
            key.1,
            r.name
        );
        reviewer_mut(t, key, round_n, &r.name).result = Some(result);
    }

    /// An agent reviewer: Claude Code is done on its Stop with the
    /// feedback file settled; Codex, which reports no Stop, when the
    /// file is there and settled. A session gone or exited without the
    /// file is a failed reviewer, never an approval. A finished
    /// reviewer's session is killed.
    fn poll_agent_reviewer(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        key: &(String, u32),
        round_n: u32,
        r: &ReviewerRun,
        now_ms: u64,
    ) -> Result<()> {
        let Some(session) = r.session.clone() else {
            return Ok(());
        };
        let view = match self.seen(&session)? {
            Seen::View(v) => *v,
            Seen::Gone => {
                reviewer_mut(t, key, round_n, &r.name).result = Some(ReviewerResult::Failed {
                    reason: "session gone".into(),
                });
                return Ok(());
            }
            Seen::Unknown => return Ok(()),
        };
        let no_feedback = stage_no_feedback(t, key);
        let rm = reviewer_mut(t, key, round_n, &r.name);
        if let Some(stop) = view.last_stop_at_ms {
            rm.stop_at_ms = Some(stop);
        }
        let running = view.liveness == wire::Liveness::Running;
        let present = rm.feedback.is_file();
        let stopped = rm.stop_at_ms.is_some()
            || matches!(view.liveness, wire::Liveness::Exited { code: Some(0) });
        let claude = rm.kind == "claude";
        if claude && !stopped {
            if !running {
                rm.result = Some(ReviewerResult::Failed {
                    reason: format!("{:?} before finishing", view.liveness),
                });
            }
            return Ok(());
        }
        // Codex reports no Stop and has no hook-driven card, so only a
        // Claude reviewer is held by a busy one.
        if claude && busy(&view) {
            rm.polls_since_stop = 0;
            return Ok(());
        }
        if !present {
            if claude {
                rm.polls_since_stop = idle_polls(&view, rm.polls_since_stop);
            }
            if !running || (claude && rm.polls_since_stop >= STOP_IDLE_POLLS) {
                rm.result = Some(ReviewerResult::Failed {
                    reason: "stopped without writing feedback".into(),
                });
            }
            return Ok(());
        }
        if !settle_file(&rm.feedback, &mut rm.settle)? {
            return Ok(());
        }
        let text = std::fs::read_to_string(&rm.feedback).unwrap_or_default();
        rm.result = Some(if text.trim() == no_feedback {
            ReviewerResult::Clean
        } else {
            ReviewerResult::Findings
        });
        log::info!(
            "ticket {} {}/{} round {round_n} reviewer {} done",
            t.id,
            key.0,
            key.1,
            r.name
        );
        self.save_ticket(t, now_ms)?;
        if running {
            self.send(
                t,
                ps,
                Some(key.clone()),
                "kill",
                Body::SessionKill { session },
                now_ms,
            )?;
        }
        Ok(())
    }

    /// A session's view, or that Switchboard says it is gone, or that
    /// the query failed for a reason that says nothing about it.
    fn seen(&mut self, session: &str) -> Result<Seen> {
        Ok(match self.session_view(session)? {
            Ok(v) => Seen::View(Box::new(v)),
            Err(reason) if reason == NO_SUCH_SESSION => Seen::Gone,
            Err(reason) => {
                log::warn!("session {session}: query failed: {reason}; asking again");
                Seen::Unknown
            }
        })
    }

    /// A fresh implementer on the round's findings.
    #[allow(clippy::too_many_arguments)]
    fn start_fix(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        key: &(String, u32),
        round: &ReviewRound,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        if round.implementer.is_some() {
            return Ok(());
        }
        let name = stage.implementer.clone().unwrap_or_default();
        let op = p.operators[&name].clone();
        let a = record_of(t, &key.0, key.1).clone();
        let rdir = round
            .feedback
            .as_ref()
            .and_then(|f| f.parent())
            .map_or_else(|| cwd.to_path_buf(), Path::to_path_buf);
        let response = rdir.join("response.md");
        let prompt = Self::fix_prompt(t, p, stage, &a, round, cwd, lane, &response, &op);
        let mut args = op.args.clone();
        args.extend(op.kind.write_flags(&rdir));
        let launch = if args.is_empty() {
            wire::Launch::Shell
        } else {
            wire::Launch::Argv(args)
        };
        let attempt = record_of(t, &key.0, key.1);
        attempt
            .artifacts
            .insert(format!("r{}/response", round.n), response.clone());
        let r = attempt
            .rounds
            .iter_mut()
            .find(|x| x.n == round.n)
            .expect("the round exists");
        r.response = Some(response);
        r.state = RoundState::Fixing;
        let notes = format!(
            "Dispatch ticket {} · #{} {} · stage {} attempt {} · round {} implementer",
            t.id,
            t.source.number.unwrap_or(0),
            t.source.title,
            key.0,
            key.1,
            round.n
        );
        let reply = self.send(
            t,
            ps,
            Some(key.clone()),
            &format!("implementer:{}", round.n),
            Body::SessionNew {
                project: a.project.unwrap_or_default(),
                name,
                session_kind: session_kind(op.kind),
                cwd: cwd.to_path_buf(),
                launch,
                prompt: Some(prompt),
                notes,
            },
            now_ms,
        )?;
        if let Reply::Failed { reason } = reply {
            let reason = format!(
                "round {}: the implementer could not start: {reason}",
                round.n
            );
            return self.fail_round(t, ps, key, round.n, &reason, now_ms);
        }
        self.unmark(t, ps, now_ms)
    }

    /// The implementer's prompt: the operator's guidance, the stage's
    /// fix template (or the default), and a note the user sent the
    /// stage back with.
    #[allow(clippy::too_many_arguments)]
    fn fix_prompt(
        t: &mut Ticket,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        round: &ReviewRound,
        cwd: &Path,
        lane: Option<&str>,
        response: &Path,
        op: &crate::pipeline::Operator,
    ) -> String {
        let mut vars = vars_for(t, p, lane);
        vars.set("base", round.base.clone())
            .set("head", round.head.clone())
            .set("worktree", cwd.display().to_string())
            .set(
                "feedback",
                round
                    .feedback
                    .as_ref()
                    .map(|f| f.display().to_string())
                    .unwrap_or_default(),
            )
            .set("response", response.display().to_string());
        if let Some(plan) = t.input("plan") {
            vars.set("plan", plan.display().to_string());
        }
        let mut prompt = guidance_prelude(&op.guidance, &vars);
        prompt.push_str(&vars.render(stage.fix_prompt.as_deref().unwrap_or(FIX_PROMPT)));
        // A record from `RECORD_VERSION` 2 or earlier keeps the note on
        // `t.rework` rather than on the attempt.
        if round.n == 1
            && let Some(note) = a
                .rework
                .clone()
                .or_else(|| t.rework.remove(&rework_key(&a.stage, &a.context)))
        {
            prompt.push_str("\n\nThe user looked at the previous attempt and sent it back: ");
            prompt.push_str(&note);
        }
        prompt
    }

    /// The implementer: done on its Stop with `response.md` settled and
    /// the tree clean and committed; then its session is killed and
    /// the checks at the new head are next.
    fn poll_fix(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        a: &Attempt,
        round: &ReviewRound,
        cwd: &Path,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let Some(session) = round.implementer.clone() else {
            return Ok(());
        };
        let view = match self.seen(&session)? {
            Seen::View(v) => *v,
            Seen::Gone => {
                let reason = format!("round {}: the implementer's session is gone", round.n);
                return self.fail_round(t, ps, &key, round.n, &reason, now_ms);
            }
            Seen::Unknown => return Ok(()),
        };
        let ticket_id = t.id.clone();
        let rm = record_of(t, &key.0, key.1)
            .rounds
            .iter_mut()
            .find(|x| x.n == round.n)
            .expect("the round exists");
        if let Some(stop) = view.last_stop_at_ms {
            rm.stop_at_ms = Some(stop);
        }
        let running = view.liveness == wire::Liveness::Running;
        let stopped = rm.stop_at_ms.is_some()
            || matches!(view.liveness, wire::Liveness::Exited { code: Some(0) });
        if !stopped {
            if running {
                return self.save_ticket(t, now_ms);
            }
            let reason = format!(
                "round {}: the implementer {:?} before finishing",
                round.n, view.liveness
            );
            return self.fail_round(t, ps, &key, round.n, &reason, now_ms);
        }
        let Some(response) = rm.response.clone() else {
            return Ok(());
        };
        if busy(&view) {
            rm.polls_since_stop = 0;
            return self.save_ticket(t, now_ms);
        }
        if !response.is_file() {
            rm.polls_since_stop = idle_polls(&view, rm.polls_since_stop);
            if rm.polls_since_stop >= STOP_IDLE_POLLS || !running {
                let reason = format!(
                    "round {}: the implementer stopped without writing {}",
                    round.n,
                    response.display()
                );
                return self.fail_round(t, ps, &key, round.n, &reason, now_ms);
            }
            return self.save_ticket(t, now_ms);
        }
        if !settle_file(&response, &mut rm.settle)? {
            return self.save_ticket(t, now_ms);
        }
        if !self.git.is_clean(cwd)? {
            // A commit whose hook is still running leaves the tree
            // dirty for minutes after the response settles; while the
            // session lives the round waits for it, within a bound.
            if running && waits_for_commit(rm, &ticket_id, &key.0, &a.context) {
                return self.save_ticket(t, now_ms);
            }
            let reason = format!(
                "round {}: the implementer left the tree at {} dirty",
                round.n,
                cwd.display()
            );
            return self.fail_round(t, ps, &key, round.n, &reason, now_ms);
        }
        let head = self.git.head(cwd)?;
        let attempt = record_of(t, &key.0, key.1);
        attempt.gate = None;
        let r = attempt
            .rounds
            .iter_mut()
            .find(|x| x.n == round.n)
            .expect("the round exists");
        r.head_after = Some(head.clone());
        r.state = RoundState::Fixed;
        log::info!(
            "ticket {} {}/{} round {} fixed; head {head}; checks next",
            t.id,
            key.0,
            a.context,
            round.n
        );
        self.save_ticket(t, now_ms)?;
        if running {
            self.send(
                t,
                ps,
                Some(key.clone()),
                "kill",
                Body::SessionKill { session },
                now_ms,
            )?;
        }
        Ok(())
    }

    /// The stage's checks at the round's current head: after a fix, or
    /// at an accepted head to complete the stage. An accepted head's
    /// checks are reused from the stage the gate names by `like` when
    /// that stage ran the same command at this clean head and passed.
    #[allow(clippy::too_many_arguments)]
    fn start_checks(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        round: &ReviewRound,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let Some(gate @ Gate::Command { .. }) = p.command_gate(stage) else {
            return self.fail_attempt(t, ps, &key.0, key.1, "no command gate", now_ms);
        };
        let argv = lane_gate_argv(gate, lane)
            .cloned()
            .filter(|v| !v.is_empty());
        let Some(argv) = argv else {
            let reason = format!("no checks command for context {}", a.context);
            return self.fail_attempt(t, ps, &key.0, key.1, &reason, now_ms);
        };
        if !self.git.is_clean(cwd)? {
            let reason = format!(
                "the tree at {} is not clean before the checks",
                cwd.display()
            );
            return self.fail_checks(t, ps, &key.0, key.1, &reason, now_ms);
        }
        let head = self.git.head(cwd)?;
        let expected = round
            .head_after
            .clone()
            .unwrap_or_else(|| round.head.clone());
        if head != expected {
            return self.park(
                t,
                ps,
                &format!(
                    "stage {} ({}): the branch moved from {expected} to {head} by something other than the implementer",
                    key.0, a.context
                ),
                now_ms,
            );
        }
        if let Some(like) = checks_reused_from(t, p, stage, a, round, lane, &head) {
            log::info!(
                "ticket {} {}/{} checks reused from {like} at {head}",
                t.id,
                key.0,
                a.context
            );
            return self.complete_review(t, ps, p, stage, &key, cwd, lane, &head, now_ms);
        }
        let log = round
            .reviewers
            .first()
            .and_then(|r| r.dir.parent())
            .map_or_else(|| cwd.join("checks.log"), |d| d.join("checks.log"));
        let env = checks_env(
            t,
            lane,
            lane.and_then(|l| t.lanes.iter().find(|x| x.name == l))
                .map(|l| l.branch.as_str()),
            a,
            cwd,
            &head,
        );
        let check_key = checks_key(t, &key, round.n);
        if let Err(e) = self.git.start_check(&check_key, cwd, &argv, &env, &log) {
            let reason = format!("the checks could not start: {e:#}");
            return self.fail_attempt(t, ps, &key.0, key.1, &reason, now_ms);
        }
        log::info!(
            "ticket {} {}/{} round {} checks started at {head}",
            t.id,
            key.0,
            a.context,
            round.n
        );
        let attempt = record_of(t, &key.0, key.1);
        attempt.gate = Some(GateRun {
            head,
            argv,
            log: log.clone(),
            started_ms: now_ms,
            exit: None,
        });
        attempt
            .artifacts
            .insert(format!("r{}/checks", round.n), log);
        self.save_ticket(t, now_ms)
    }

    /// The checks' exit, bound to the head only if the tree is still
    /// clean at it: after a fix, the next round opens; at an accepted
    /// head, the stage completes. A lost child (a restart) starts again
    /// on the same clean head, as a command gate does.
    #[allow(clippy::too_many_arguments)]
    fn poll_checks(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        round: &ReviewRound,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let Some(gate) = a.gate.clone() else {
            return Ok(());
        };
        let check_key = checks_key(t, &key, round.n);
        let code = match self.git.poll_check(&check_key) {
            None => return Ok(()),
            Some(Ok(code)) => code,
            Some(Err(e)) => {
                log::warn!(
                    "ticket {} {}/{} checks lost ({e:#}); starting again",
                    t.id,
                    key.0,
                    a.context
                );
                record_of(t, &key.0, key.1).gate = None;
                let a = record_of(t, &key.0, key.1).clone();
                return self.start_checks(t, ps, p, stage, &a, round, cwd, lane, now_ms);
            }
        };
        let clean = self.git.is_clean(cwd)?;
        let head = self.git.head(cwd)?;
        if let Some(g) = &mut record_of(t, &key.0, key.1).gate {
            g.exit = Some(code);
        }
        if !clean || head != gate.head {
            let reason = format!(
                "the tree at {} changed while the checks ran (head {} then {head})",
                cwd.display(),
                gate.head
            );
            return self.fail_checks(t, ps, &key.0, key.1, &reason, now_ms);
        }
        if code != 0 {
            let reason = crate::scheduler::checks_reason(code, &gate.log);
            return self.fail_checks(t, ps, &key.0, key.1, &reason, now_ms);
        }
        log::info!(
            "ticket {} {}/{} round {} checks passed at {head}",
            t.id,
            key.0,
            a.context,
            round.n
        );
        if round.state == RoundState::Fixed {
            record_of(t, &key.0, key.1).gate = None;
            self.save_ticket(t, now_ms)?;
            return self.open_round(t, ps, p, stage, &key, cwd, lane, now_ms);
        }
        self.complete_review(t, ps, p, stage, &key, cwd, lane, &head, now_ms)
    }

    /// The round and the attempt failed for one reason; the rerun
    /// question follows.
    fn fail_round(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        key: &(String, u32),
        round_n: u32,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        set_round_state(
            t,
            key,
            round_n,
            RoundState::Failed {
                reason: reason.to_owned(),
            },
            now_ms,
        );
        self.fail_attempt(t, ps, &key.0, key.1, reason, now_ms)
    }

    /// The stage's checks passed at `head`: the commits are rewritten
    /// as the stage says, and the attempt completes at the head that
    /// leaves.
    #[allow(clippy::too_many_arguments)]
    fn complete_review(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        key: &(String, u32),
        cwd: &Path,
        lane: Option<&str>,
        head: &str,
        now_ms: u64,
    ) -> Result<()> {
        let Some(head) = self.rewrite_commits(t, ps, p, stage, key, cwd, lane, head, now_ms)?
        else {
            return Ok(());
        };
        self.finish_review(t, key, &head, now_ms)
    }

    /// The summary written and the attempt complete at `head`.
    fn finish_review(
        &mut self,
        t: &mut Ticket,
        key: &(String, u32),
        head: &str,
        now_ms: u64,
    ) -> Result<()> {
        let a = record_of(t, &key.0, key.1).clone();
        let path = self
            .attempt_dir(t, &key.0, key.1, &a.context)?
            .join("summary.md");
        let carry = carry_of(t, &a, &stage_no_feedback(t, key));
        std::fs::write(&path, summary_of(&a, carry.as_ref(), head))
            .with_context(|| format!("write {}", path.display()))?;
        let attempt = record_of(t, &key.0, key.1);
        attempt.artifacts.insert("summary".into(), path);
        attempt.head = Some(head.to_owned());
        attempt.state = AttemptState::Complete;
        attempt.ended_ms = Some(now_ms);
        if let Some(r) = attempt.rounds.last_mut() {
            r.ended_ms = Some(now_ms);
        }
        log::info!("ticket {} {}/{} complete at {head}", t.id, key.0, key.1);
        self.save_ticket(t, now_ms)
    }

    /// A `keep` answer to a failed rewrite: the attempt completes at the
    /// head its checks passed at, with the history as it is. Nothing runs
    /// again, so each thing the checks proved is checked to still hold.
    pub(crate) fn keep_history(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        attempt: Option<&(String, u32)>,
        now_ms: u64,
    ) -> Result<()> {
        let Some(key) = attempt.cloned() else {
            return Ok(());
        };
        // An attempt that is no longer the failed rewrite the question
        // was about: the answer was overtaken.
        let Some(a) = find_attempt(t, &key.0, key.1)
            .filter(|a| matches!(a.state, AttemptState::Failed { .. }))
            .cloned()
        else {
            return Ok(());
        };
        let Some(before) = a.rewrite.as_ref().map(|r| r.before.clone()) else {
            return Ok(());
        };
        let Some(cwd) = crate::scheduler::tree_of(t, p, &a.context) else {
            return self.park(
                t,
                ps,
                &format!(
                    "stage {} ({}): keep was answered but the context has no tree",
                    key.0, a.context
                ),
                now_ms,
            );
        };
        // The branch must be where the checks passed, or keeping it
        // keeps something nobody checked: it parks, as a stale review
        // answer does.
        let head = self.git.head(&cwd)?;
        if head != before {
            return self.park(
                t,
                ps,
                &format!(
                    "stage {} ({}): keep was answered for head {before} but the branch is at {head}; the answer is stale",
                    key.0, a.context
                ),
                now_ms,
            );
        }
        // The rewrite runs only after the checks pass; that is read
        // from the record, since nothing is run again here.
        let lane = t
            .lanes
            .iter()
            .any(|l| l.name == a.context)
            .then(|| a.context.clone());
        let own = a
            .gate
            .as_ref()
            .is_some_and(|g| g.head == before && g.exit == Some(0));
        let passed = own
            || p.stages
                .iter()
                .find(|s| s.name == a.stage)
                .zip(a.rounds.last())
                .is_some_and(|(stage, round)| {
                    checks_reused_from(t, p, stage, &a, round, lane.as_deref(), &before).is_some()
                });
        if !passed {
            return self.park(
                t,
                ps,
                &format!(
                    "stage {} ({}): keep was answered but the checks did not pass at {before}",
                    key.0, a.context
                ),
                now_ms,
            );
        }
        // A tree changed since is not what the checks saw. The branch
        // is still at the reviewed head, so `keep` stays on offer for
        // once the tree is cleaned.
        if !self.git.is_clean(&cwd)? {
            let reason = format!(
                "the tree at {} is not clean, so the commits at {before} cannot be kept",
                cwd.display()
            );
            return self.fail_rewrite(t, ps, &key.0, key.1, &reason, now_ms);
        }
        if let Some(r) = &mut record_of(t, &key.0, key.1).rewrite {
            r.skipped = Some(KEPT_BY_HAND.to_owned());
            r.after = Some(before.clone());
        }
        log::info!(
            "ticket {} {}/{} commits kept by hand at {before}",
            t.id,
            key.0,
            a.context
        );
        self.finish_review(t, &key, &before, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// The branch's commits at `before` rewritten as the stage's
    /// `commits` says, on a clean tree, into a head with the same tree,
    /// and the branch moved there only while it is still at `before`.
    /// The head the attempt completes at, or `None` when the attempt
    /// failed (or the ticket parked) instead. A branch the remote
    /// already holds is left as it is.
    #[allow(clippy::too_many_arguments)]
    fn rewrite_commits(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        key: &(String, u32),
        cwd: &Path,
        lane: Option<&str>,
        before: &str,
        now_ms: u64,
    ) -> Result<Option<String>> {
        let mode = stage.commits();
        if mode == Commits::Keep {
            return Ok(Some(before.to_owned()));
        }
        if !self.git.is_clean(cwd)? {
            let reason = format!(
                "the tree at {} is not clean when its commits would be rewritten",
                cwd.display()
            );
            self.fail_attempt(t, ps, &key.0, key.1, &reason, now_ms)?;
            return Ok(None);
        }
        let base = self.base_of(t, p, cwd, lane, now_ms)?;
        let commits = match self.git.commits(cwd, &base, before) {
            Ok(c) => c,
            Err(e) => {
                let reason = format!("the commits since {base} could not be read: {e:#}");
                self.fail_attempt(t, ps, &key.0, key.1, &reason, now_ms)?;
                return Ok(None);
            }
        };
        let from = count(commits.len());
        let mut record = Rewrite {
            mode,
            before: before.to_owned(),
            after: None,
            from,
            to: from,
            skipped: None,
            at_ms: now_ms,
        };
        if self.is_published(t, p, cwd, lane, key, &base, before)? {
            log::info!(
                "ticket {} {}/{} commits kept at {before}: {PUBLISHED}",
                t.id,
                key.0,
                key.1
            );
            record.after = Some(before.to_owned());
            record.skipped = Some(PUBLISHED.to_owned());
            record_of(t, &key.0, key.1).rewrite = Some(record);
            return Ok(Some(before.to_owned()));
        }
        let context = record_of(t, &key.0, key.1).context.clone();
        let ranges = fix_ranges(t, &key.0, &context);
        let planned = match mode {
            Commits::One => history::one_plan(&commits, &base, &ranges),
            Commits::Fold | Commits::Keep => history::fold_plan(&commits, &base, &ranges),
        };
        let groups = match planned {
            Ok(g) => g,
            Err(e) => {
                // On the attempt, so a `keep` answer knows the head the
                // checks passed at.
                record_of(t, &key.0, key.1).rewrite = Some(record);
                self.fail_rewrite(t, ps, &key.0, key.1, &format!("{e:#}"), now_ms)?;
                return Ok(None);
            }
        };
        if history::is_identity(&commits, &groups) {
            record.after = Some(before.to_owned());
            record_of(t, &key.0, key.1).rewrite = Some(record);
            return Ok(Some(before.to_owned()));
        }
        // The intent goes on the record before git writes anything, so a
        // restart knows a move may have landed.
        record.to = count(groups.len());
        record_of(t, &key.0, key.1).rewrite = Some(record);
        self.save_ticket(t, now_ms)?;
        self.replay_and_move(t, ps, key, cwd, &base, before, &groups, now_ms)
    }

    /// `groups` replayed onto `base` and the branch moved from `before`
    /// to the result, proven to have the same tree before and after the
    /// move; moved back if it does not. The new head, or `None` when the
    /// attempt failed (or the ticket parked).
    #[allow(clippy::too_many_arguments)]
    fn replay_and_move(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        key: &(String, u32),
        cwd: &Path,
        base: &str,
        before: &str,
        groups: &[history::Group],
        now_ms: u64,
    ) -> Result<Option<String>> {
        let fail = |this: &mut Self, t: &mut Ticket, ps: &mut ProjectState, reason: String| {
            this.fail_rewrite(t, ps, &key.0, key.1, &reason, now_ms)
                .map(|()| None)
        };
        let after = match self.git.replay(cwd, base, groups) {
            Ok(a) => a,
            Err(e) => {
                return fail(self, t, ps, format!("{e:#}; the branch stays at {before}"));
            }
        };
        let want = self.git.tree(cwd, before)?;
        let got = self.git.tree(cwd, &after)?;
        if let Some(r) = &mut record_of(t, &key.0, key.1).rewrite {
            r.after = Some(after.clone());
        }
        if got != want {
            return fail(
                self,
                t,
                ps,
                format!(
                    "the rewritten head {after} has tree {got}, not {want} as {before} has; the branch stays at {before}"
                ),
            );
        }
        if let Err(e) = self.git.set_head(cwd, &after, before) {
            return fail(
                self,
                t,
                ps,
                format!("the branch could not be moved from {before} to {after}: {e:#}"),
            );
        }
        let head = self.git.head(cwd)?;
        let clean = self.git.is_clean(cwd)?;
        let tree = self.git.tree(cwd, "HEAD")?;
        if head != after || !clean || tree != want {
            let problem = format!(
                "after the move from {before} to {after} the tree at {} reads head {head}, tree {tree}{}",
                cwd.display(),
                if clean { "" } else { ", not clean" }
            );
            if let Err(e) = self.git.set_head(cwd, before, &after) {
                self.park(
                    t,
                    ps,
                    &format!(
                        "stage {} ({}): {problem}, and the branch could not be moved back to {before}: {e:#}",
                        key.0,
                        cwd.display()
                    ),
                    now_ms,
                )?;
                return Ok(None);
            }
            return fail(
                self,
                t,
                ps,
                format!("{problem}; the branch is back at {before}"),
            );
        }
        if let Some(r) = find_attempt(t, &key.0, key.1).and_then(|a| a.rewrite.as_ref()) {
            log::info!(
                "ticket {} rewrote {before} → {after} ({}, {} → {} commits)",
                t.id,
                r.mode.as_str(),
                r.from,
                r.to
            );
        }
        Ok(Some(after))
    }

    /// Whether the branch at `cwd` is on the remote already: a refresh
    /// pushed it, a pull request was looked up for it, or the remote's
    /// copy of the branch holds a commit of `base..head` (an agent's
    /// push records nothing else).
    #[allow(clippy::too_many_arguments)]
    fn is_published(
        &self,
        t: &Ticket,
        p: &Pipeline,
        cwd: &Path,
        lane: Option<&str>,
        key: &(String, u32),
        base: &str,
        head: &str,
    ) -> Result<bool> {
        let record = lane.and_then(|l| t.lanes.iter().find(|x| x.name == l));
        let context = find_attempt(t, &key.0, key.1).map(|a| a.context.as_str());
        if record.is_some_and(|l| l.pushed.is_some())
            || t.attempts
                .iter()
                .any(|a| Some(a.context.as_str()) == context && a.pr.is_some())
        {
            return Ok(true);
        }
        let remote = lane
            .and_then(|l| p.lane(l))
            .map_or(p.project.remote.as_str(), |l| p.lane_remote(l));
        self.git.published(cwd, remote, base, head)
    }

    /// A rewrite whose intent was saved before a restart: the branch is
    /// still at `before` (the move never landed) and the rewrite runs
    /// again; or it is at another head with the same clean tree (the
    /// move landed and the save did not) and the attempt completes
    /// there; or anything else fails the attempt.
    #[allow(clippy::too_many_arguments)]
    fn resume_rewrite(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        a: &Attempt,
        before: &str,
        cwd: &Path,
        lane: Option<&str>,
        now_ms: u64,
    ) -> Result<()> {
        let key = (a.stage.clone(), a.n);
        let head = self.git.head(cwd)?;
        if head == before {
            log::info!(
                "ticket {} {}/{} rewrite at {before} never moved the branch; rewriting again",
                t.id,
                key.0,
                a.context
            );
            return self.complete_review(t, ps, p, stage, &key, cwd, lane, before, now_ms);
        }
        let clean = self.git.is_clean(cwd)?;
        let tree = self.git.tree(cwd, "HEAD")?;
        let want = self.git.tree(cwd, before)?;
        if clean && tree == want {
            log::info!(
                "ticket {} {}/{} rewrite from {before} landed at {head}",
                t.id,
                key.0,
                a.context
            );
            if let Some(r) = &mut record_of(t, &key.0, key.1).rewrite {
                r.after = Some(head.clone());
            }
            return self.finish_review(t, &key, &head, now_ms);
        }
        let reason = format!(
            "the rewrite from {before} was interrupted and the tree at {} is at head {head}, tree {tree}{}, not {want}",
            cwd.display(),
            if clean { "" } else { ", not clean" }
        );
        self.fail_attempt(t, ps, &key.0, key.1, &reason, now_ms)
    }

    /// An answer to a round's question, on the attempt's last round:
    /// `fix` authorises the fix pass, `accept` takes the reviewed head
    /// (the checks run at it next), `more` at the cap authorises one
    /// fix pass and one review pass beyond it. A head that moved since
    /// the question makes the answer stale: the ticket parks with the
    /// two heads named, as any unexpected movement does.
    pub(crate) fn review_answer(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        name: &str,
        answer: &str,
        attempt: Option<&(String, u32)>,
        now_ms: u64,
    ) -> Result<()> {
        let Some(key) = attempt.cloned() else {
            return Ok(());
        };
        let Some(a) = find_attempt(t, &key.0, key.1).cloned() else {
            return Ok(());
        };
        let Some(round) = a.rounds.last().cloned() else {
            return Ok(());
        };
        if round.state != RoundState::Findings {
            log::warn!(
                "ticket {} {}/{}: {name} answered {answer} but round {} is not waiting",
                t.id,
                key.0,
                key.1,
                round.n
            );
            return Ok(());
        }
        let cwd =
            crate::scheduler::tree_of(t, &self.pipeline_of(t)?, &a.context).unwrap_or_default();
        let head = self.git.head(&cwd)?;
        if head != round.head {
            return self.park(
                t,
                ps,
                &format!(
                    "stage {} ({}): {name} was answered for head {} but the branch is at {head}; the answer is stale",
                    key.0, a.context, round.head
                ),
                now_ms,
            );
        }
        let attempt = record_of(t, &key.0, key.1);
        let r = attempt
            .rounds
            .iter_mut()
            .find(|x| x.n == round.n)
            .expect("the round exists");
        match answer {
            "fix" => r.fix_authorised = true,
            "more" => {
                r.fix_authorised = true;
                attempt.extra_pass = true;
            }
            _ => r.state = RoundState::Accepted,
        }
        self.save_ticket(t, now_ms)?;
        self.unmark(t, ps, now_ms)
    }

    /// Every command reviewer and check of the attempt that this runner
    /// started and that may still run, killed.
    pub(crate) fn kill_review_commands(&mut self, t: &Ticket, a: &Attempt) {
        let key = (a.stage.clone(), a.n);
        for round in &a.rounds {
            for r in &round.reviewers {
                if r.kind == "command" && r.launched && r.result.is_none() {
                    self.git
                        .kill_check(&reviewer_key(t, &key, round.n, &r.name));
                }
            }
            if a.gate.as_ref().is_some_and(|g| g.exit.is_none()) {
                self.git.kill_check(&checks_key(t, &key, round.n));
            }
        }
    }
}

/// A round judged: the round file's text, the points that hold it
/// open, the style points left to the merge and the points that contest
/// the plan (each as its listed line).
struct Aggregated {
    text: String,
    open: u32,
    left: Vec<String>,
    decided: Vec<String>,
}

/// How a point counts towards convergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tag {
    /// Wording, naming, comments or layout: holds the early rounds only.
    Style,
    /// Contests the plan's decisions: never open, listed as not done.
    Decided,
    /// Holds the round open.
    Other,
}

/// A point's tag, by its text's case-insensitive prefix.
fn tag_of(text: &str) -> Tag {
    let text = text.trim_start().to_ascii_lowercase();
    if text.starts_with("style:") {
        Tag::Style
    } else if text.starts_with("decided:") {
        Tag::Decided
    } else {
        Tag::Other
    }
}

/// Whether a reviewer's line declares that every point it has left is
/// wording, so the round can close: a note about the round, not a
/// point. The declaration's clause must end at "wording" (or "wording
/// is left") and hold no "not", "except" or "but", and only a closing
/// remark such as "the round can close on it" may follow it, so a real
/// point never hides behind the declaration. A line quoting code names
/// something to change, so it is a point.
fn declares_wording(text: &str) -> bool {
    let text = text.trim();
    let text = match text.get(..6) {
        Some(tag) if tag.eq_ignore_ascii_case("style:") => &text[6..],
        _ => text,
    };
    let text = text.trim().to_ascii_lowercase();
    if text.contains('`') {
        return false;
    }
    let (first, rest) = text.split_once([';', '.', ',', ':']).unwrap_or((&text, ""));
    let words: Vec<&str> = first.split_whitespace().collect();
    let ends_at_wording = words
        .iter()
        .rposition(|w| *w == "wording")
        .is_some_and(|i| {
            words[i + 1..]
                .iter()
                .all(|w| matches!(*w, "is" | "left" | "remains" | "remaining"))
        });
    let declares = ends_at_wording
        && !words.iter().any(|w| matches!(*w, "not" | "except" | "but"))
        && (first.contains("left") || first.contains("remain"))
        && [
            "every point",
            "every remaining point",
            "points left",
            "remaining points",
            "only wording",
            "all wording",
        ]
        .iter()
        .any(|p| first.contains(p));
    let rest = rest.trim_matches(|c: char| c.is_whitespace() || matches!(c, ';' | '.' | ',' | ':'));
    let rest = rest
        .strip_prefix("so ")
        .or_else(|| rest.strip_prefix("and "))
        .unwrap_or(rest);
    declares
        && matches!(
            rest,
            "" | "the round can close on it" | "the round can close" | "it can close"
        )
}

/// A point's tag, with an untagged point from the reviewer named
/// `style` counted as style: convergence must not depend on that
/// reviewer remembering the prefix.
fn class_of(reviewer: &str, text: &str) -> Tag {
    match tag_of(text) {
        Tag::Other if reviewer == STYLE_REVIEWER => Tag::Style,
        tag => tag,
    }
}

/// The reviewer a point id names: `r2/style-1` and `a1/r2/style-1` are
/// both `style`'s.
fn reviewer_of(id: &str) -> &str {
    let last = id.rsplit('/').next().unwrap_or(id);
    last.rsplit_once('-').map_or(last, |(name, _)| name)
}

/// The findings of a round gathered into one file: every reviewer's
/// points with its name and a stable id, then the points open coming
/// into the round (`earlier`) that no reviewer withdrew. A point that
/// contests the plan is never open; when every point left is style and
/// the round, counted as `counted`, has reached `style_rounds`, they
/// are left to the merge and the round converges.
fn aggregate(
    t: &Ticket,
    stage: &Stage,
    ctx: &str,
    round: &ReviewRound,
    earlier: &[(String, String)],
    counted: u32,
    style_rounds: u32,
) -> Aggregated {
    let no_feedback = no_feedback_of(stage);
    let (points, withdrawn, kept, notes) = collect_points(round, &no_feedback);
    let carried: Vec<&(String, String)> = earlier
        .iter()
        .filter(|(id, _)| !withdrawn.contains(id))
        .collect();
    let (decided, raised): (Vec<_>, Vec<_>) = points
        .into_iter()
        .partition(|(_, _, text)| tag_of(text) == Tag::Decided);
    let classes: Vec<Tag> = raised
        .iter()
        .map(|(_, r, text)| class_of(r, text))
        .chain(
            carried
                .iter()
                .map(|(id, text)| class_of(reviewer_of(id), text)),
        )
        .collect();
    let style = classes.iter().filter(|c| **c == Tag::Style).count();
    let blocking = classes.len() - style;
    let to_merge = blocking == 0 && style > 0 && counted >= style_rounds;
    let raised_lines: Vec<String> = raised
        .iter()
        .map(|(id, r, text)| format!("{id} ({r}): {text}"))
        .collect();
    let carried_lines: Vec<String> = carried
        .iter()
        .map(|(id, text)| carried_line(id, text, &kept))
        .collect();
    let decided_lines: Vec<String> = decided
        .iter()
        .map(|(id, r, text)| format!("{id} ({r}): {text}"))
        .collect();
    let (open, left) = if to_merge {
        (
            0,
            raised_lines.iter().chain(&carried_lines).cloned().collect(),
        )
    } else {
        (classes.len(), Vec::new())
    };
    let note_lines: Vec<String> = notes
        .iter()
        .map(|(r, text)| format!("{r}: {text}"))
        .collect();
    let mut out = String::new();
    let _ = writeln!(out, "# Review round {} — {} ({ctx})\n", round.n, stage.name);
    let branch = t
        .lanes
        .iter()
        .find(|l| l.name == ctx)
        .map(|l| l.branch.clone())
        .unwrap_or_default();
    let _ = writeln!(
        out,
        "Branch {branch} at {}, over {}.\n",
        round.head, round.base
    );
    if open == 0 && left.is_empty() && decided_lines.is_empty() {
        let _ = writeln!(out, "{no_feedback}");
    } else {
        if open == 0 {
            let _ = writeln!(out, "No open point.");
        } else {
            section(&mut out, "Points", None, &raised_lines);
            section(
                &mut out,
                "Still open from earlier rounds",
                None,
                &carried_lines,
            );
        }
        section(&mut out, LEFT_HEADING, None, &left);
        section(
            &mut out,
            DECIDED_HEADING,
            Some(
                "These contest the plan's decisions; they are out of scope for this review and not for the fix pass.",
            ),
            &decided_lines,
        );
    }
    section(
        &mut out,
        NOTES_HEADING,
        Some("Not points: what a reviewer said of the round."),
        &note_lines,
    );
    Aggregated {
        text: out,
        open: u32::try_from(open).unwrap_or(u32::MAX),
        left,
        decided: decided_lines,
    }
}

/// A section of a round file, written only when it lists something.
fn section(out: &mut String, title: &str, intro: Option<&str>, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    let _ = writeln!(out, "## {title}\n");
    if let Some(intro) = intro {
        let _ = writeln!(out, "{intro}\n");
    }
    for line in lines {
        let _ = writeln!(out, "- {line}");
    }
}

/// A point still open from an earlier round, with who kept it and why.
fn carried_line(id: &str, text: &str, kept: &[(String, String, String)]) -> String {
    let why: Vec<String> = kept
        .iter()
        .filter(|(k, _, _)| k == id)
        .map(|(_, r, w)| {
            if w.is_empty() {
                r.clone()
            } else {
                format!("{r}: {w}")
            }
        })
        .collect();
    if why.is_empty() {
        format!("{id}: {text}")
    } else {
        format!("{id}: {text} (kept by {})", why.join("; "))
    }
}

/// Each reviewer's points (id, reviewer, text), the ids withdrawn,
/// the ids kept with who kept them and why, and each reviewer's notes
/// (reviewer, text): lines declaring that every point left is wording.
type Points = (
    Vec<(String, String, String)>,
    Vec<String>,
    Vec<(String, String, String)>,
    Vec<(String, String)>,
);

fn collect_points(round: &ReviewRound, no_feedback: &str) -> Points {
    let mut points: Vec<(String, String, String)> = Vec::new();
    let mut withdrawn: Vec<String> = Vec::new();
    let mut kept: Vec<(String, String, String)> = Vec::new();
    let mut notes: Vec<(String, String)> = Vec::new();
    for r in &round.reviewers {
        if r.result != Some(ReviewerResult::Findings) {
            continue;
        }
        let text = std::fs::read_to_string(&r.feedback).unwrap_or_default();
        if text.trim() == no_feedback {
            continue;
        }
        let mut k = 0;
        let mut listed = false;
        // The file's text without its declarations, for a file that
        // lists nothing: a declaration must not swallow the prose
        // beside it.
        let mut prose = String::new();
        for raw in text.lines() {
            let line = raw.trim();
            let said = point_text(line).unwrap_or_else(|| line.to_owned());
            if declares_wording(&said) {
                notes.push((r.name.clone(), said));
                continue;
            }
            prose.push_str(raw);
            prose.push('\n');
            // Reviewers write their lines as list items as often as
            // not; a bulleted `withdraw` or `keep` is the same ruling.
            let ruling = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .map_or(line, str::trim);
            if let Some(id) = ruling.strip_prefix("withdraw ") {
                withdrawn.push(id.trim().to_owned());
                listed = true;
            } else if let Some(rest) = ruling.strip_prefix("keep ") {
                let (id, why) = rest.split_once(':').unwrap_or((rest, ""));
                kept.push((id.trim().to_owned(), r.name.clone(), why.trim().to_owned()));
                listed = true;
            } else if let Some(point) = point_text(line) {
                k += 1;
                points.push((
                    format!("r{}/{}-{k}", round.n, r.name),
                    r.name.clone(),
                    point,
                ));
                listed = true;
            }
        }
        if !listed && !prose.trim().is_empty() {
            points.push((
                format!("r{}/{}-1", round.n, r.name),
                r.name.clone(),
                prose.trim().replace('\n', " "),
            ));
        }
    }
    (points, withdrawn, kept, notes)
}

/// What the previous round left open: its points the implementer
/// disputed (or did not answer).
fn carried_points(previous: Option<&ReviewRound>) -> Vec<(String, String)> {
    previous
        .map(answered_points)
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, _, fixed)| !fixed)
        .map(|(id, text, _)| (id, text))
        .collect()
}

/// A gathered round's open points, each with whether its response
/// answers it as fixed. Empty for a round with no feedback.
fn answered_points(round: &ReviewRound) -> Vec<(String, String, bool)> {
    let Some(feedback) = &round.feedback else {
        return Vec::new();
    };
    let answered = round
        .response
        .as_ref()
        .map(|r| std::fs::read_to_string(r).unwrap_or_default())
        .unwrap_or_default();
    open_points_of(&std::fs::read_to_string(feedback).unwrap_or_default())
        .into_iter()
        .map(|(id, text)| {
            let fixed = claims_fixed(&answered, &id);
            (id, text, fixed)
        })
        .collect()
}

/// Whether a response answers point `id` as fixed.
fn claims_fixed(response: &str, id: &str) -> bool {
    response.lines().any(|l| {
        let l = l.trim().trim_start_matches("- ");
        l.starts_with(id)
            && l[id.len()..]
                .trim_start_matches(':')
                .trim()
                .starts_with("fixed")
    })
}

/// A list item's text, for `- `, `* ` and `1. ` lines.
fn point_text(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
        return Some(rest.trim().to_owned());
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0
        && let Some(rest) = line[digits..].strip_prefix(". ")
    {
        return Some(rest.trim().to_owned());
    }
    None
}

/// The point ids and texts an aggregated feedback file holds open:
/// every section before the points left to the merge, those found but
/// not done, and the reviewers' notes.
fn open_points_of(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if [LEFT_HEADING, DECIDED_HEADING, NOTES_HEADING]
            .iter()
            .any(|h| line == format!("## {h}"))
        {
            break;
        }
        out.extend(listed_point(line));
    }
    out
}

/// Every point id and text an aggregated feedback file lists, in any
/// section.
fn listed_points_of(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(listed_point).collect()
}

/// The points one section of an aggregated feedback file lists, as
/// their id and their whole line.
fn section_points(text: &str, title: &str) -> Vec<(String, String)> {
    let heading = format!("## {title}");
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("## ") {
            inside = line == heading;
        } else if inside && let Some((id, _)) = listed_point(line) {
            out.push((id, line.trim_start_matches("- ").to_owned()));
        }
    }
    out
}

/// One listed point of an aggregated feedback file: `- <id>: text` or
/// `- <id> (reviewer): text`.
fn listed_point(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("- ")?;
    let (id, text) = rest.split_once(':')?;
    let id = id.split(" (").next().unwrap_or(id).trim();
    is_point_id(id).then(|| (id.to_owned(), text.trim().to_owned()))
}

/// A point id: `r<round>/<reviewer>-<k>`, or one carried from an
/// earlier attempt, `a<n>/r<round>/<reviewer>-<k>`.
fn is_point_id(id: &str) -> bool {
    let rest = unqualified(id).unwrap_or(id);
    rest.strip_prefix('r')
        .and_then(|r| r.split_once('/'))
        .is_some_and(|(n, name)| {
            !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && !name.is_empty()
        })
}

/// An id's rest after its `a<n>/` qualifier, when it has one.
fn unqualified(id: &str) -> Option<&str> {
    let (q, rest) = id.split_once('/')?;
    let n = q.strip_prefix('a')?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(rest)
}

/// A point id as a later attempt names it: qualified with the attempt
/// it was raised in, since ids are unique only within an attempt. An id
/// carried twice keeps its first qualifier.
fn qualify(n: u32, id: &str) -> String {
    if unqualified(id).is_some() {
        id.to_owned()
    } else {
        format!("a{n}/{id}")
    }
}

/// What a new attempt of a code review continues: an earlier attempt of
/// the same stage and context whose rounds read the branch.
struct Carry {
    from: (String, u32),
    /// The last round of that attempt with gathered findings.
    through: u32,
    /// The base and head that round read.
    base: String,
    head: String,
    /// The points it left open, and those settled before, qualified.
    open: Vec<(String, String)>,
    settled: Vec<(String, String)>,
}

/// The attempt a new attempt `n` of a review in `ctx` carries: the
/// latest earlier one if any round of it gathered findings, otherwise
/// whatever that one carried, so a chain of failures points at the
/// attempt that holds the state.
fn carry_source(t: &Ticket, stage: &str, ctx: &str, n: u32) -> Option<(String, u32)> {
    let prev = previous_review(t, stage, ctx, n)?;
    if prev.rounds.iter().any(|r| r.feedback.is_some()) {
        Some((stage.to_owned(), prev.n))
    } else {
        prev.carried_from.clone()
    }
}

/// The review attempt of `stage` in `ctx` just before attempt `n`.
fn previous_review<'t>(t: &'t Ticket, stage: &str, ctx: &str, n: u32) -> Option<&'t Attempt> {
    t.attempts
        .iter()
        .filter(|a| {
            a.stage == stage && a.context == ctx && a.n < n && a.kind == AttemptKind::Review
        })
        .max_by_key(|a| a.n)
}

/// The note the attempt just before `n` was given and no fixer was:
/// it failed before its first fix pass. Only that attempt is looked at,
/// so a note a fixer spent is never given again.
fn unspent_note(t: &Ticket, stage: &str, ctx: &str, n: u32) -> Option<String> {
    let prev = previous_review(t, stage, ctx, n)?;
    if prev.rounds.iter().any(|r| r.response.is_some()) {
        return None;
    }
    prev.rework.clone()
}

/// A note that asks for the whole branch to be reviewed again.
fn starts_over(note: &str) -> bool {
    note.to_lowercase().contains("start over")
}

/// What attempt `a` carries, rebuilt from the carried attempt's round
/// files and records alone, so a restart reads the same.
fn carry_of(t: &Ticket, a: &Attempt, no_feedback: &str) -> Option<Carry> {
    let (stage, m) = a.carried_from.clone()?;
    let src = find_attempt(t, &stage, m)?;
    let last = src.rounds.iter().rev().find(|r| r.feedback.is_some())?;
    // A round after the last gathered one (the one that failed) may
    // still have withdrawn points.
    let withdrawn: Vec<String> = src
        .rounds
        .iter()
        .filter(|r| r.n > last.n)
        .flat_map(|r| collect_points(r, no_feedback).1)
        .collect();
    // A fix the attempt answered but no later round read is not
    // settled: the attempt may have failed on exactly that fix. Such a
    // point stays open, marked for its fix to be checked.
    let open: Vec<(String, String)> = answered_points(last)
        .into_iter()
        .filter(|(id, _, _)| !withdrawn.contains(id))
        .map(|(id, text, fixed)| {
            let text = if fixed {
                format!("{text} {UNCHECKED_FIX}")
            } else {
                text
            };
            (qualify(m, &id), text)
        })
        .collect();
    let mut settled = Vec::new();
    if let Some(before) = carry_of(t, src, no_feedback) {
        settled.extend(before.settled);
        settled.extend(before.open);
    }
    for path in src.rounds.iter().filter_map(|r| r.feedback.as_ref()) {
        let text = std::fs::read_to_string(path).unwrap_or_default();
        settled.extend(
            listed_points_of(&text)
                .into_iter()
                .map(|(id, text)| (qualify(m, &id), text)),
        );
    }
    let mut seen: std::collections::BTreeSet<String> =
        open.iter().map(|(id, _)| id.clone()).collect();
    settled.retain(|(id, _)| seen.insert(id.clone()));
    Some(Carry {
        from: (stage, m),
        through: last.n,
        base: last.base.clone(),
        head: last.head.clone(),
        open,
        settled,
    })
}

/// The carried attempt's state, told to round 1's reviewers: self-
/// contained, since the old round files name the points without their
/// qualifier.
fn carried_text(c: &Carry, round: &ReviewRound) -> String {
    let list = |points: &[(String, String)]| {
        if points.is_empty() {
            "None.".to_owned()
        } else {
            points
                .iter()
                .map(|(id, text)| format!("- {id}: {text}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    let mut vars = Vars::default();
    vars.set("carried_n", c.from.1.to_string())
        .set("old_head", c.head.clone())
        .set("head", round.head.clone())
        .set("through", c.through.to_string())
        .set("settled", list(&c.settled))
        .set("open", list(&c.open));
    let range = if c.base == round.base {
        format!("git diff {} {}", c.head, round.head)
    } else {
        format!(
            "git range-diff {}..{} {}..{}, the branch was rebased since",
            c.base, c.head, round.base, round.head
        )
    };
    vars.set("range", range);
    let scope = if c.head == round.head {
        REVIEW_CARRIED_STILL.to_owned()
    } else {
        vars.render(REVIEW_CARRIED_RANGE)
    };
    vars.set("scope", scope);
    vars.render(REVIEW_CARRIED)
}

/// The lane's last bring-up, when this round is the first review of the
/// rebase it made: the branch had commits, the round reads the new
/// base, and no round with findings of this stage and context has read
/// that base before.
fn rebase_to_check<'t>(
    t: &'t Ticket,
    a: &Attempt,
    round: &ReviewRound,
    lane: Option<&str>,
) -> Option<&'t Refreshed> {
    let moved = t
        .lanes
        .iter()
        .find(|l| Some(l.name.as_str()) == lane)?
        .refreshed
        .as_ref()?;
    if !moved.commits || moved.to != round.base {
        return None;
    }
    let read_before = t
        .attempts
        .iter()
        .filter(|x| x.stage == a.stage && x.context == a.context && x.n <= a.n)
        .flat_map(|x| x.rounds.iter().map(move |r| (x.n, r)))
        .any(|(n, r)| !(n == a.n && r.n == round.n) && r.feedback.is_some() && r.base == moved.to);
    (!read_before).then_some(moved)
}

/// The rebase check, with the rebaser's notes when there are some.
fn rebased_text(moved: &Refreshed) -> String {
    let mut vars = Vars::default();
    vars.set("from", moved.from.clone())
        .set("to", moved.to.clone());
    let mut text = vars.render(if moved.from.is_empty() {
        REVIEW_REBASED_UNKNOWN
    } else {
        REVIEW_REBASED
    });
    if let Some(notes) = &moved.notes {
        let _ = write!(text, " The rebaser's notes are at {}.", notes.display());
    }
    text
}

/// The plan's decisions section: a heading of any level whose title,
/// after an optional number such as `2.`, starts with the word
/// "Decisions", through to the next heading of the same or a higher
/// level. Its body, without the heading.
fn decisions_section(plan: &str) -> Option<String> {
    let mut level = None;
    let mut fenced = false;
    let mut body = Vec::new();
    for line in plan.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let heading = if fenced { None } else { heading_of(line) };
        match (level, heading) {
            (None, Some((lv, title))) if is_decisions(title) => {
                level = Some(lv);
                continue;
            }
            (Some(l), Some((lv, _))) if lv <= l => break,
            _ => {}
        }
        if level.is_some() {
            body.push(line);
        }
    }
    level.map(|_| body.join("\n").trim().to_owned())
}

/// A Markdown heading's level and title.
fn heading_of(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let title = line[hashes..].strip_prefix(' ')?;
    Some((hashes, title.trim()))
}

/// Whether a heading's title is a decisions section's.
fn is_decisions(title: &str) -> bool {
    let digits = title.chars().take_while(char::is_ascii_digit).count();
    let title = if digits > 0 {
        title[digits..]
            .strip_prefix('.')
            .unwrap_or(&title[digits..])
            .trim_start()
    } else {
        title
    };
    let lower = title.to_ascii_lowercase();
    lower
        .strip_prefix("decisions")
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !c.is_alphanumeric()))
}

/// The fix rounds' `(head, head_after)` of every review attempt of
/// `stage` in `ctx`, so a carried rerun's earlier fixes still fold.
fn fix_ranges(t: &Ticket, stage: &str, ctx: &str) -> Vec<(String, String)> {
    t.attempts
        .iter()
        .filter(|a| a.stage == stage && a.context == ctx && a.kind == AttemptKind::Review)
        .flat_map(|a| &a.rounds)
        .filter_map(|r| Some((r.head.clone(), r.head_after.clone()?)))
        .collect()
}

/// The stage the gate names by `like`, when its completed run of the
/// same command at `head` stands for attempt `a`'s checks: only at a
/// head `round` accepted. `round` is the attempt's last round.
pub(crate) fn checks_reused_from<'s>(
    t: &Ticket,
    p: &Pipeline,
    stage: &'s Stage,
    a: &Attempt,
    round: &ReviewRound,
    lane: Option<&str>,
    head: &str,
) -> Option<&'s str> {
    let Some(Gate::Command {
        like: Some(like), ..
    }) = &stage.gate
    else {
        return None;
    };
    let argv = p
        .command_gate(stage)
        .and_then(|g| lane_gate_argv(g, lane))
        .filter(|v| !v.is_empty())?;
    let accepting = matches!(round.state, RoundState::Converged | RoundState::Accepted);
    (accepting
        && t.attempts.iter().any(|x| {
            &x.stage == like
                && x.context == a.context
                && x.state == AttemptState::Complete
                && x.gate
                    .as_ref()
                    .is_some_and(|g| g.head == head && g.exit == Some(0) && &g.argv == argv)
        }))
    .then_some(like.as_str())
}

/// A length as a record's count.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The summary a completed code review attempt leaves: how it ended,
/// what it carried, the style points left to the merge, the points open
/// when it was accepted, and the points that contest the plan.
fn summary_of(a: &Attempt, carry: Option<&Carry>, head: &str) -> String {
    let read = |r: &ReviewRound| {
        r.feedback
            .as_ref()
            .map(|f| std::fs::read_to_string(f).unwrap_or_default())
            .unwrap_or_default()
    };
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Review summary — {} ({}), attempt {}\n",
        a.stage, a.context, a.n
    );
    let last = a.rounds.last();
    let k = last.map_or(0, |r| r.n);
    let accepted = last.is_some_and(|r| r.state == RoundState::Accepted);
    if accepted {
        let m = last.map_or(0, |r| r.open_points);
        let _ = writeln!(
            out,
            "Accepted at `{head}` in round {k} with {m} point(s) open."
        );
    } else {
        let _ = writeln!(out, "Converged at `{head}` in round {k}.");
    }
    if let Some(r) = &a.rewrite {
        match (&r.skipped, &r.after) {
            (Some(why), _) => {
                let _ = writeln!(out, "Commits kept: {why}.");
            }
            (None, Some(after)) if *after != r.before => {
                let _ = if r.mode == Commits::One {
                    writeln!(
                        out,
                        "Squashed {} commits to one: `{}` → `{after}`.",
                        r.from, r.before
                    )
                } else {
                    writeln!(
                        out,
                        "Commits folded from {} to {}: `{}` → `{after}`.",
                        r.from, r.to, r.before
                    )
                };
            }
            _ => {}
        }
    }
    if let Some(c) = carry {
        let _ = writeln!(
            out,
            "Carried from attempt {}, read through round {} at `{}`.",
            c.from.1, c.through, c.head
        );
    }
    let last_text = last.map(read).unwrap_or_default();
    let lines = |points: Vec<(String, String)>| -> Vec<String> {
        points.into_iter().map(|(_, line)| line).collect()
    };
    summary_section(
        &mut out,
        LEFT_HEADING,
        &lines(section_points(&last_text, LEFT_HEADING)),
    );
    if accepted {
        let open: Vec<String> = open_points_of(&last_text)
            .into_iter()
            .map(|(id, text)| format!("{id}: {text}"))
            .collect();
        summary_section(&mut out, "Open when accepted", &open);
    }
    let mut seen = std::collections::BTreeSet::new();
    let decided: Vec<String> = a
        .rounds
        .iter()
        .flat_map(|r| section_points(&read(r), DECIDED_HEADING))
        .filter(|(id, _)| seen.insert(id.clone()))
        .map(|(_, line)| line)
        .collect();
    summary_section(&mut out, DECIDED_HEADING, &decided);
    out
}

/// A summary section; `None.` when it lists nothing.
fn summary_section(out: &mut String, title: &str, lines: &[String]) {
    let _ = writeln!(out, "\n## {title}\n");
    if lines.is_empty() {
        let _ = writeln!(out, "None.");
    }
    for line in lines {
        let _ = writeln!(out, "- {line}");
    }
}

/// The stage's sentinel for "no findings", from the stage the caller
/// holds.
fn no_feedback_of(stage: &Stage) -> String {
    stage
        .no_feedback
        .clone()
        .unwrap_or_else(|| NO_FINDINGS.to_owned())
}

/// Counts a poll with the tree still dirty after the response settled.
/// True while the round should keep waiting for the commit to land.
fn waits_for_commit(rm: &mut ReviewRound, ticket: &str, stage: &str, context: &str) -> bool {
    if rm.dirty_polls >= DIRTY_POLLS {
        return false;
    }
    if rm.dirty_polls == 0 {
        log::info!(
            "ticket {ticket} {stage}/{context} round {}: the tree is dirty after the response; waiting for a commit",
            rm.n
        );
    }
    rm.dirty_polls += 1;
    true
}

fn reviewer_mut<'a>(
    t: &'a mut Ticket,
    key: &(String, u32),
    round_n: u32,
    name: &str,
) -> &'a mut ReviewerRun {
    record_of(t, &key.0, key.1)
        .rounds
        .iter_mut()
        .find(|r| r.n == round_n)
        .expect("the round exists")
        .reviewers
        .iter_mut()
        .find(|r| r.name == name)
        .expect("the reviewer exists")
}

fn set_round_state(
    t: &mut Ticket,
    key: &(String, u32),
    round_n: u32,
    state: RoundState,
    now_ms: u64,
) {
    if let Some(r) = record_of(t, &key.0, key.1)
        .rounds
        .iter_mut()
        .find(|r| r.n == round_n)
    {
        let ended = matches!(
            state,
            RoundState::Converged | RoundState::Accepted | RoundState::Failed { .. }
        );
        r.state = state;
        if ended {
            r.ended_ms = Some(now_ms);
        }
    }
}

/// The stage's sentinel, read from the ticket's pipeline copy; the
/// default when the copy cannot be read. For callers that hold only
/// the attempt's key, not its `Stage`, such as the completion summary.
fn stage_no_feedback(t: &Ticket, key: &(String, u32)) -> String {
    std::fs::read_to_string(&t.pipeline_file)
        .ok()
        .and_then(|text| Pipeline::parse(&text).ok())
        .and_then(|p| {
            p.stages
                .iter()
                .find(|s| s.name == key.0)
                .and_then(|s| s.no_feedback.clone())
        })
        .unwrap_or_else(|| NO_FINDINGS.to_owned())
}

/// The key a command reviewer's child is polled under.
fn reviewer_key(t: &Ticket, key: &(String, u32), round_n: u32, name: &str) -> String {
    format!("{}/{}/{}/r{round_n}/{name}", t.id, key.0, key.1)
}

/// The key a round's checks are polled under.
fn checks_key(t: &Ticket, key: &(String, u32), round_n: u32) -> String {
    format!("{}/{}/{}/r{round_n}/checks", t.id, key.0, key.1)
}

/// A reply to a reviewer's or an implementer's `session.new`, applied
/// to its round: the intent names the round and the reviewer.
pub(crate) fn apply_review_reply(t: &mut Ticket, intent: &str, made: &[wire::Made]) {
    let Some(op) = t.ledger.iter().rev().find(|o| o.intent == intent) else {
        return;
    };
    let Some(key) = op.attempt.clone() else {
        return;
    };
    let Some(id) = made
        .iter()
        .find(|m| m.kind == wire::RecordKind::Session)
        .map(|m| m.id.clone())
    else {
        return;
    };
    let Some(attempt) = find_attempt_mut(t, &key.0, key.1) else {
        return;
    };
    if let Some(rest) = intent.strip_prefix("reviewer:")
        && let Some((n, name)) = rest.split_once(':')
        && let Ok(n) = n.parse::<u32>()
        && let Some(round) = attempt.rounds.iter_mut().find(|r| r.n == n)
        && let Some(r) = round.reviewers.iter_mut().find(|r| r.name == name)
    {
        r.session = Some(id.clone());
        t.processes.push(id);
    } else if let Some(n) = intent.strip_prefix("implementer:")
        && let Ok(n) = n.parse::<u32>()
        && let Some(round) = attempt.rounds.iter_mut().find(|r| r.n == n)
    {
        round.implementer = Some(id.clone());
        t.processes.push(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_decisions_section_is_found_by_its_heading() {
        let plan = "# Plan\n\n## Decisions\n\n1. Keep X.\n### Why\nBecause Y.\n## Tests\n- t1\n";
        assert_eq!(
            decisions_section(plan).as_deref(),
            Some("1. Keep X.\n### Why\nBecause Y.")
        );
        let numbered = "## 2. Decisions that must be made before starting (recommendations given)\n- D1\n# Next\n";
        assert_eq!(decisions_section(numbered).as_deref(), Some("- D1"));
        assert_eq!(
            decisions_section("## Decision for the human: which lane\n- a\n"),
            None
        );
        assert_eq!(
            decisions_section("## 3. Decision cancellation is not a thing\n- a\n"),
            None
        );
        assert_eq!(decisions_section("# Plan\n## Steps\n- s1\n"), None);
        let fenced = "## Decisions\n- D1\n```sh\n# not a heading\n```\n- D2\n## After\n";
        assert_eq!(
            decisions_section(fenced).as_deref(),
            Some("- D1\n```sh\n# not a heading\n```\n- D2")
        );
    }

    #[test]
    fn a_point_is_tagged_by_its_prefix_and_the_style_reviewer_by_default() {
        assert_eq!(tag_of("style: rename tmp"), Tag::Style);
        assert_eq!(tag_of("Style: rename tmp"), Tag::Style);
        assert_eq!(tag_of("DECIDED: D1 should be Z"), Tag::Decided);
        assert_eq!(tag_of("src/a.rs: unused import"), Tag::Other);
        assert_eq!(tag_of("a style: point in the middle"), Tag::Other);
        assert_eq!(class_of("style", "src/x.rs: too clever"), Tag::Style);
        assert_eq!(class_of("style", "decided: D1"), Tag::Decided);
        assert_eq!(class_of("lint", "style: wording"), Tag::Style);
        assert_eq!(class_of("lint", "src/a.rs: unused"), Tag::Other);
        assert_eq!(reviewer_of("r2/style-1"), "style");
        assert_eq!(reviewer_of("a1/r2/code-review-3"), "code-review");
    }

    #[test]
    fn point_ids_may_carry_an_attempt_qualifier_once() {
        assert!(is_point_id("r1/lint-1"));
        assert!(is_point_id("a3/r12/style-2"));
        assert!(!is_point_id("rename/this"));
        assert!(!is_point_id("r/lint-1"));
        assert!(!is_point_id("a/r1/lint-1"));
        assert!(!is_point_id("src/a.rs"));
        assert_eq!(qualify(1, "r1/lint-1"), "a1/r1/lint-1");
        assert_eq!(qualify(2, "a1/r1/lint-1"), "a1/r1/lint-1");
    }

    #[test]
    fn open_points_stop_at_the_merge_and_not_done_sections() {
        let text = "# Review round 3 — review-code (repo)\n\nBranch b at h, over b.\n\n## Points\n\n- r3/lint-1 (lint): src/a.rs: unused\n\n## Still open from earlier rounds\n\n- a1/r1/lint-2: src/b.rs: dead\n\n## Left to the merge\n\n- r3/style-1 (style): style: wording\n\n## Found but not done\n\nThese contest the plan's decisions.\n\n- r3/style-2 (style): decided: D1 should be Z\n";
        let ids = |points: Vec<(String, String)>| -> Vec<String> {
            points.into_iter().map(|(id, _)| id).collect()
        };
        assert_eq!(ids(open_points_of(text)), ["r3/lint-1", "a1/r1/lint-2"]);
        assert_eq!(
            ids(listed_points_of(text)),
            ["r3/lint-1", "a1/r1/lint-2", "r3/style-1", "r3/style-2"]
        );
        assert_eq!(
            section_points(text, LEFT_HEADING),
            [(
                "r3/style-1".to_owned(),
                "r3/style-1 (style): style: wording".to_owned()
            )]
        );
    }

    #[test]
    fn a_line_declaring_every_point_left_is_wording_is_a_note() {
        for yes in [
            "Every point left is wording; the round can close on it.",
            "style: every point I have left is wording",
            "Only wording is left.",
            "All remaining points are wording.",
            "every point left is wording, so the round can close on it",
        ] {
            assert!(declares_wording(yes), "{yes}");
        }
        for no in [
            "style: rename tmp",
            "style: the doc comment's wording on `free_name` is off",
            "the wording left in the README is stale",
            "style: every point left is wording; also rename tmp",
            "style: every point left is wording, but rename tmp to buf",
            "every point left is wording: rename tmp",
            "Every point left is wording except the unwrap in scheduler.rs poll_gate",
            "Not every point left is wording",
        ] {
            assert!(!declares_wording(no), "{no}");
        }
    }

    fn round_said(dir: &std::path::Path, text: &str) -> ReviewRound {
        let feedback = dir.join("feedback.md");
        std::fs::write(&feedback, text).unwrap();
        ReviewRound {
            n: 3,
            base: "b".into(),
            head: "h".into(),
            reviewers: vec![ReviewerRun {
                name: "style".into(),
                kind: "claude".into(),
                dir: dir.to_owned(),
                feedback,
                session: None,
                launched: true,
                stop_at_ms: None,
                polls_since_stop: 0,
                settle: None,
                result: Some(ReviewerResult::Findings),
            }],
            state: RoundState::Reviewing,
            feedback: None,
            open_points: 0,
            fix_authorised: false,
            implementer: None,
            response: None,
            head_after: None,
            stop_at_ms: None,
            polls_since_stop: 0,
            settle: None,
            dirty_polls: 0,
            started_ms: 0,
            ended_ms: None,
        }
    }

    #[test]
    fn a_declaration_is_collected_as_a_note_and_takes_no_number() {
        let dir = tempfile::tempdir().unwrap();
        let declaration = "Every point left is wording; the round can close on it.";
        let note = vec![("style".to_owned(), declaration.to_owned())];
        let point = |text: &str| ("r3/style-1".to_owned(), "style".to_owned(), text.to_owned());

        let round = round_said(dir.path(), &format!("- {declaration}\n- style: x\n"));
        let (points, _, _, notes) = collect_points(&round, "NO_FEEDBACK");
        assert_eq!(points, [point("style: x")]);
        assert_eq!(notes, note);

        let round = round_said(dir.path(), &format!("{declaration}\n"));
        let (points, _, _, notes) = collect_points(&round, "NO_FEEDBACK");
        assert!(points.is_empty());
        assert_eq!(notes, note);

        let prose = "The doc comment on free_name says the wrong thing.";
        let round = round_said(dir.path(), &format!("{declaration}\n{prose}\n"));
        let (points, _, _, notes) = collect_points(&round, "NO_FEEDBACK");
        assert_eq!(points, [point(prose)]);
        assert_eq!(notes, note);
    }
}
