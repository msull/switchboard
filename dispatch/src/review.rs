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

use crate::pipeline::{Gate, OperatorKind, Pipeline, Stage};
use crate::scheduler::{
    Ask, NO_SUCH_SESSION, Runner, SocketDown, asks_again, env_for, held_in, may_rerun, new_attempt,
    next_n, primary_tree, rework_key, session_kind, vars_for,
};
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, DecisionKind, GateRun, ProjectState, ReviewRound,
    ReviewerResult, ReviewerRun, RoundState, SETTLE_POLLS, Settle, Ticket, TicketState,
};

/// What a reviewer writes when it has nothing to report, unless the
/// stage says otherwise.
pub const NO_FINDINGS: &str = "No findings.";

/// The reviewer prompt a stage gets when it gives none.
const REVIEW_PROMPT: &str = "Review branch {branch} in {worktree}: the commits {base}..{head} (see git diff {base} {head}). Write your findings to {feedback} as a Markdown list, one point per line starting with \"- \", each naming the file and saying why it matters. If you find nothing, write exactly this line alone: {no_feedback}. Change nothing in {worktree}.";

/// The addition when earlier rounds left points open.
const REVIEW_CARRY: &str = "Points still open from earlier rounds are listed at {previous_feedback} with the implementer's answers at {previous_response}. For each open point write a line \"withdraw <id>\" if the answer satisfies you, or \"keep <id>: why\" if it does not.";

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
        let contexts = Self::contexts(t, p, stage);
        if contexts.is_empty() {
            return self.park(
                t,
                ps,
                &format!("stage {} needs lanes the ticket has not cut", stage.name),
                now_ms,
            );
        }
        let mut all_complete = true;
        for (ctx, cwd, lane) in contexts {
            let last = t
                .attempts
                .iter()
                .filter(|a| a.stage == stage.name && a.context == ctx)
                .max_by_key(|a| a.n)
                .cloned();
            match last {
                Some(a) if a.state == AttemptState::Complete => {}
                Some(a) if a.is_open() => {
                    all_complete = false;
                    self.poll_review(t, ps, p, stage, &a, &cwd, lane.as_deref(), now_ms)?;
                }
                Some(a) => {
                    all_complete = false;
                    let held = held_in(t, &stage.name, &ctx);
                    let sent_back = t.rework.contains_key(&rework_key(&stage.name, &ctx));
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
            self.advance(t, ps, now_ms)?;
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
        t.attempts.push(attempt);
        // A note from a later human gate goes to the first implementer
        // of this attempt, not to the reviewers.
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
        let a = attempt_mut(t, key).clone();
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
        let attempt = attempt_mut(t, key);
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
        let a = attempt_mut(t, key).clone();
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
    /// head, the tree, the plan and the earlier rounds' open points.
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
        let mut prompt = String::new();
        let guidance = p.operators[&r.name].guidance.trim();
        if !guidance.is_empty() {
            prompt.push_str(&vars.render(guidance));
            prompt.push_str("\n\n");
        }
        prompt.push_str(&vars.render(stage.review_prompt.as_deref().unwrap_or(REVIEW_PROMPT)));
        if carried {
            prompt.push_str("\n\n");
            prompt.push_str(&vars.render(REVIEW_CARRY));
        }
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
                if a.gate.is_some() {
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
        let round = attempt_mut(t, &key)
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
        let previous = a.rounds.iter().rev().find(|x| x.n < round.n).cloned();
        let (text, open) = aggregate(t, stage, &a.context, &round, previous.as_ref());
        let path = round
            .reviewers
            .first()
            .and_then(|r| r.dir.parent())
            .map_or_else(|| cwd.join("feedback.md"), |d| d.join("feedback.md"));
        std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
        {
            let attempt = attempt_mut(t, &key);
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
        }
        if open == 0 {
            log::info!(
                "ticket {} {}/{} round {} converged at {head}",
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
            let a = attempt_mut(t, key).clone();
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
            attempt_mut(t, &key)
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
        if !present {
            rm.polls_since_stop += 1;
            if !running || (claude && rm.polls_since_stop >= SETTLE_POLLS) {
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
        let a = attempt_mut(t, key).clone();
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
        {
            let attempt = attempt_mut(t, key);
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
        }
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
        let mut prompt = String::new();
        let guidance = op.guidance.trim();
        if !guidance.is_empty() {
            prompt.push_str(&vars.render(guidance));
            prompt.push_str("\n\n");
        }
        prompt.push_str(&vars.render(stage.fix_prompt.as_deref().unwrap_or(FIX_PROMPT)));
        if round.n == 1
            && let Some(note) = t.rework.remove(&rework_key(&a.stage, &a.context))
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
        let rm = attempt_mut(t, &key)
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
        if !response.is_file() {
            rm.polls_since_stop += 1;
            if rm.polls_since_stop >= SETTLE_POLLS || !running {
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
            let reason = format!(
                "round {}: the implementer left the tree at {} dirty",
                round.n,
                cwd.display()
            );
            return self.fail_round(t, ps, &key, round.n, &reason, now_ms);
        }
        let head = self.git.head(cwd)?;
        {
            let attempt = attempt_mut(t, &key);
            attempt.gate = None;
            let r = attempt
                .rounds
                .iter_mut()
                .find(|x| x.n == round.n)
                .expect("the round exists");
            r.head_after = Some(head.clone());
            r.state = RoundState::Fixed;
        }
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
        let Some(Gate::Command { argv, per_lane, .. }) = p.command_gate(stage) else {
            return self.fail_attempt(t, ps, &key.0, key.1, "no command gate", now_ms);
        };
        let argv = lane
            .and_then(|l| per_lane.as_ref().and_then(|m| m.get(l)))
            .or(argv.as_ref())
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
        let accepting = matches!(round.state, RoundState::Converged | RoundState::Accepted);
        if accepting
            && let Some(Gate::Command {
                like: Some(like), ..
            }) = &stage.gate
            && t.attempts.iter().any(|x| {
                &x.stage == like
                    && x.context == a.context
                    && x.state == AttemptState::Complete
                    && x.gate
                        .as_ref()
                        .is_some_and(|g| g.head == head && g.exit == Some(0) && g.argv == argv)
            })
        {
            log::info!(
                "ticket {} {}/{} checks reused from {like} at {head}",
                t.id,
                key.0,
                a.context
            );
            return self.complete_review(t, &key, &head, now_ms);
        }
        let log = round
            .reviewers
            .first()
            .and_then(|r| r.dir.parent())
            .map_or_else(|| cwd.join("checks.log"), |d| d.join("checks.log"));
        let mut env = env_for(
            t,
            lane,
            lane.and_then(|l| t.lanes.iter().find(|x| x.name == l))
                .map(|l| l.branch.as_str()),
        );
        env.push(("DISPATCH_STAGE".to_owned(), key.0.clone()));
        env.push(("DISPATCH_CONTEXT".to_owned(), a.context.clone()));
        env.push(("DISPATCH_TREE".to_owned(), cwd.display().to_string()));
        env.push(("DISPATCH_HEAD".to_owned(), head.clone()));
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
        let attempt = attempt_mut(t, &key);
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
                attempt_mut(t, &key).gate = None;
                let a = attempt_mut(t, &key).clone();
                return self.start_checks(t, ps, p, stage, &a, round, cwd, lane, now_ms);
            }
        };
        let clean = self.git.is_clean(cwd)?;
        let head = self.git.head(cwd)?;
        if let Some(g) = &mut attempt_mut(t, &key).gate {
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
            attempt_mut(t, &key).gate = None;
            self.save_ticket(t, now_ms)?;
            return self.open_round(t, ps, p, stage, &key, cwd, lane, now_ms);
        }
        self.complete_review(t, &key, &head, now_ms)
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

    fn complete_review(
        &mut self,
        t: &mut Ticket,
        key: &(String, u32),
        head: &str,
        now_ms: u64,
    ) -> Result<()> {
        let attempt = attempt_mut(t, key);
        attempt.head = Some(head.to_owned());
        attempt.state = AttemptState::Complete;
        attempt.ended_ms = Some(now_ms);
        if let Some(r) = attempt.rounds.last_mut() {
            r.ended_ms = Some(now_ms);
        }
        log::info!("ticket {} {}/{} complete at {head}", t.id, key.0, key.1);
        self.save_ticket(t, now_ms)
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
        let Some(a) = t
            .attempts
            .iter()
            .find(|a| a.stage == key.0 && a.n == key.1)
            .cloned()
        else {
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
        let attempt = attempt_mut(t, &key);
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

/// The findings of a round gathered into one file: every reviewer's
/// points with its name and a stable id, then the points earlier
/// rounds left open that no reviewer withdrew. Returns the text and
/// the number of open points.
fn aggregate(
    t: &Ticket,
    stage: &Stage,
    ctx: &str,
    round: &ReviewRound,
    previous: Option<&ReviewRound>,
) -> (String, u32) {
    let no_feedback = stage.no_feedback.as_deref().unwrap_or(NO_FINDINGS);
    let (points, withdrawn, kept) = collect_points(round, no_feedback);
    let carried = carried_points(previous, &withdrawn);
    let open = u32::try_from(points.len() + carried.len()).unwrap_or(u32::MAX);
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
    if open == 0 {
        let _ = writeln!(out, "{no_feedback}");
        return (out, 0);
    }
    let _ = writeln!(out, "## Points\n");
    for (id, reviewer, text) in &points {
        let _ = writeln!(out, "- {id} ({reviewer}): {text}");
    }
    if !carried.is_empty() {
        let _ = writeln!(out, "\n## Still open from earlier rounds\n");
        for (id, text) in &carried {
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
            let _ = writeln!(
                out,
                "- {id}: {text}{}",
                if why.is_empty() {
                    String::new()
                } else {
                    format!(" (kept by {})", why.join("; "))
                }
            );
        }
    }
    (out, open)
}

/// Each reviewer's points (id, reviewer, text), the ids withdrawn,
/// and the ids kept with who kept them and why.
type Points = (
    Vec<(String, String, String)>,
    Vec<String>,
    Vec<(String, String, String)>,
);

fn collect_points(round: &ReviewRound, no_feedback: &str) -> Points {
    let mut points: Vec<(String, String, String)> = Vec::new();
    let mut withdrawn: Vec<String> = Vec::new();
    let mut kept: Vec<(String, String, String)> = Vec::new();
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
        for line in text.lines() {
            let line = line.trim();
            if let Some(id) = line.strip_prefix("withdraw ") {
                withdrawn.push(id.trim().to_owned());
                listed = true;
            } else if let Some(rest) = line.strip_prefix("keep ") {
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
        if !listed && !text.trim().is_empty() {
            points.push((
                format!("r{}/{}-1", round.n, r.name),
                r.name.clone(),
                text.trim().replace('\n', " "),
            ));
        }
    }
    (points, withdrawn, kept)
}

/// What the previous round left open: its points the implementer
/// disputed (or did not answer), minus those withdrawn now.
fn carried_points(previous: Option<&ReviewRound>, withdrawn: &[String]) -> Vec<(String, String)> {
    let mut carried: Vec<(String, String)> = Vec::new();
    if let Some(prev) = previous
        && let Some(feedback) = &prev.feedback
    {
        let answered = prev
            .response
            .as_ref()
            .map(|r| std::fs::read_to_string(r).unwrap_or_default())
            .unwrap_or_default();
        for (id, text) in open_points_of(&std::fs::read_to_string(feedback).unwrap_or_default()) {
            let fixed = answered.lines().any(|l| {
                let l = l.trim().trim_start_matches("- ");
                l.starts_with(&id)
                    && l[id.len()..]
                        .trim_start_matches(':')
                        .trim()
                        .starts_with("fixed")
            });
            if fixed || withdrawn.iter().any(|w| w == &id) {
                continue;
            }
            carried.push((id, text));
        }
    }
    carried
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

/// The point ids and texts an aggregated feedback file lists.
fn open_points_of(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let rest = line.trim().strip_prefix("- ")?;
            let (id, text) = rest.split_once(':')?;
            let id = id.split(" (").next().unwrap_or(id).trim();
            if !id.starts_with('r') || !id.contains('/') {
                return None;
            }
            Some((id.to_owned(), text.trim().to_owned()))
        })
        .collect()
}

/// Whether a file has looked the same for `SETTLE_POLLS` polls.
fn settle_file(path: &Path, settle: &mut Option<Settle>) -> Result<bool> {
    let meta = std::fs::metadata(path)?;
    let mtime_ms = crate::epoch_ms(meta.modified()?);
    let len = meta.len();
    let entry = settle.get_or_insert(Settle {
        mtime_ms,
        len,
        polls: 0,
    });
    if entry.mtime_ms == mtime_ms && entry.len == len {
        entry.polls += 1;
    } else {
        *entry = Settle {
            mtime_ms,
            len,
            polls: 1,
        };
    }
    Ok(entry.polls >= SETTLE_POLLS)
}

/// The attempt `(stage, n)` names.
fn attempt_mut<'a>(t: &'a mut Ticket, key: &(String, u32)) -> &'a mut Attempt {
    t.attempts
        .iter_mut()
        .find(|a| a.stage == key.0 && a.n == key.1)
        .expect("the attempt exists")
}

fn reviewer_mut<'a>(
    t: &'a mut Ticket,
    key: &(String, u32),
    round_n: u32,
    name: &str,
) -> &'a mut ReviewerRun {
    attempt_mut(t, key)
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
    if let Some(r) = attempt_mut(t, key)
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
/// default when the copy cannot be read.
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
    let Some(attempt) = t
        .attempts
        .iter_mut()
        .find(|a| a.stage == key.0 && a.n == key.1)
    else {
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

/// Whether the ticket is parked or closing, for callers that poll.
#[allow(dead_code)]
fn stopped(t: &Ticket) -> bool {
    !matches!(t.state, TicketState::Active)
}
