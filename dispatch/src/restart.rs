//! Restarting a ticket: put it at its stage, or an earlier one, under a
//! fresh copy of the project's live pipeline. The restart rides on a
//! park, so everything running is read back as gone before anything
//! moves; then the branches go back to the heads recorded as the ticket
//! entered the stage, or stay where they are when nothing after the
//! target can have moved them or they are at their base; the later work
//! is discarded, and the stage asks before any agent of it runs again.
//! The target may be a stage only the live file has, when every live
//! stage before it was run.
//!
//! A gate-only stage's command is taken not to commit, so its attempt
//! never makes a reset needed. A commit such a command does make is kept
//! on the branch and recorded in the entry the restart writes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};

use crate::pipeline::{Gate, Lane, Pipeline};
use crate::scheduler::{REFRESH, RESOLUTION, Runner, rework_key, tree_branch};
use crate::ticket::{
    Attempt, AttemptKind, AttemptState, DecisionState, HeadReset, LaneAtEntry, Restart,
    RestartIntent, StageEntry, Ticket, TicketState,
};

/// The start of the reason a restart cancels a completed attempt with.
pub const DISCARDED_BY: &str = "discarded by restart at ";

/// A restart that passed its checks: the live file and where the
/// ticket stands in each copy.
struct Checked {
    /// The live file's text, written as the new copy.
    text: String,
    new: Pipeline,
    /// The stage the ticket stands at, by name.
    from: String,
    /// The stage it is put at, by name.
    to: String,
    /// The first stage of the ticket's copy whose work is discarded:
    /// `to` itself, or the stage after a live-only `to`.
    target_old: usize,
    /// `to`'s index in the live file.
    target_new: usize,
    /// `to` is earlier than the current stage, or only the live file has
    /// it: the work from `target_old` on is discarded, and the branches
    /// are reset where needed.
    ranged: bool,
    /// The stages whose work is discarded, by name: from `target_old` on
    /// in the ticket's copy for a ranged restart, the current stage alone
    /// for a plain one.
    range: Vec<String>,
    /// The newest entry recorded for `target_old` on a ranged restart:
    /// the heads the branches go back to and the lane state put back.
    entry: Option<StageEntry>,
    /// The branches of a ranged restart and the head each goes back to;
    /// empty for a plain one.
    targets: Vec<Target>,
}

/// A branch a ranged restart moves: its key in `StageEntry::heads`, its
/// tree, the branch checked out there, and the head it goes back to,
/// which is its current head when no reset is needed.
struct Target {
    key: String,
    dir: PathBuf,
    branch: String,
    to: String,
}

impl Runner {
    /// Put a ticket at `stage` (its current one when `None`) under a
    /// fresh copy of the project's live pipeline. Checked before
    /// anything is written; then the intent is saved with `Parking`, and
    /// the park sequence applies it once every process is read back as
    /// gone, now or on a later pass. The ticket comes back as it stands:
    /// active with the restart applied, still parking, or parked with
    /// why the restart is held.
    pub fn restart(&mut self, ticket: &str, stage: Option<&str>, now_ms: u64) -> Result<Ticket> {
        self.transaction(|r| {
            let mut t = r.load_ticket(ticket)?;
            match &t.state {
                TicketState::Closing { .. } | TicketState::Closed { .. } => bail!(
                    "ticket {ticket} is {}; that is a retake: close it and take the issue again",
                    t.state.label()
                ),
                TicketState::Parking { .. } if t.restart.is_none() => {
                    bail!("ticket {ticket} is still parking; try again when it is parked")
                }
                _ => {}
            }
            let old = r.pipeline_of(&t)?;
            let checked = r.check_restart(&t, &old, stage)?;
            let carried = match &t.restart {
                Some(intent) => {
                    let earlier = target_name(&t, &old, intent.stage.as_deref());
                    if earlier == checked.to {
                        intent.reset.clone()
                    } else if intent.reset.is_empty() {
                        Vec::new()
                    } else {
                        let done: Vec<String> =
                            intent.reset.iter().map(ToString::to_string).collect();
                        bail!(
                            "ticket {ticket}: a restart at {earlier} has already reset {}; finish it with `dispatch restart {ticket} {earlier}`, or close the ticket",
                            done.join(", ")
                        );
                    }
                }
                None => Vec::new(),
            };
            t.restart = Some(RestartIntent {
                stage: stage.map(str::to_owned),
                made_ms: now_ms,
                reset: carried,
            });
            let mut ps = r.load_project(&t.project)?;
            let before = ps.clone();
            let parked = r.park(&mut t, &mut ps, &format!("restarting at {}", checked.to), now_ms);
            if ps != before {
                r.save_project(&ps)?;
            }
            parked?;
            Ok(t)
        })
    }

    /// Everything a restart is refused for, read before anything is
    /// written: the live file, the stage named, the lanes and project
    /// unchanged, and for a ranged restart the heads to reset to.
    fn check_restart(&self, t: &Ticket, old: &Pipeline, stage: Option<&str>) -> Result<Checked> {
        let path = if t.source.is_pull_request() {
            self.data.pr_pipeline(&t.project)
        } else {
            self.data.pipeline(&t.project)
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("the live pipeline {} cannot be read", path.display()))?;
        let new = Pipeline::parse(&text)
            .with_context(|| format!("the live pipeline {} does not parse", path.display()))?;
        if new.project.name != t.project {
            bail!(
                "the live pipeline {} names project {}, not {}",
                path.display(),
                new.project.name,
                t.project
            );
        }
        let Some(from) = old.stages.get(t.stage).map(|s| s.name.clone()) else {
            bail!("ticket {} is past its last stage", t.id);
        };
        let to = stage.map_or_else(|| from.clone(), str::to_owned);
        let (target_old, target_new, live_only) = locate(t, old, &new, &from, &to)?;
        for lane in &t.lanes {
            let Some(live) = new.lane(&lane.name) else {
                bail!(
                    "the live pipeline drops lane {}, which this ticket has",
                    lane.name
                );
            };
            if old.lane(&lane.name).map(lane_shape) != Some(lane_shape(live)) {
                bail!(
                    "the live pipeline changes lane {}'s path, repo, base or remote",
                    lane.name
                );
            }
        }
        let (a, b) = (&old.project, &new.project);
        if (&a.repo, &a.base, &a.remote, &a.worktrees)
            != (&b.repo, &b.base, &b.remote, &b.worktrees)
        {
            bail!("the live pipeline changes the project's repo, base, remote or worktrees");
        }
        let ranged = target_old < t.stage || live_only;
        let mut targets = Vec::new();
        let mut range = vec![from.clone()];
        let mut entry = None;
        if ranged {
            if !old.cuts_worktrees() {
                bail!("the project works in place, so there is no branch to reset to {to}");
            }
            if t.source.is_pull_request() {
                bail!(
                    "ticket {} reviews someone else's branches; restart it at its current stage",
                    t.id
                );
            }
            range = old.stages[target_old..]
                .iter()
                .map(|s| s.name.clone())
                .collect();
            entry = latest_entry(t, &old.stages[target_old].name).cloned();
            targets = self
                .reset_targets(t, old, &to, &range, entry.as_ref())
                .map_err(anyhow::Error::msg)?;
        }
        Ok(Checked {
            text,
            new,
            from,
            to,
            target_old,
            target_new,
            ranged,
            range,
            entry,
            targets,
        })
    }

    /// The restart a park carried, applied once every process is read
    /// back as gone. A check the live file now fails, or a branch git
    /// will not move, parks the ticket with why and keeps the intent:
    /// `dispatch restart` again carries it on, from the resets already
    /// saved.
    pub(crate) fn apply_restart(&mut self, t: &mut Ticket, now_ms: u64) -> Result<()> {
        let Some(intent) = t.restart.clone() else {
            return Ok(());
        };
        let old = match self.pipeline_of(t) {
            Ok(p) => p,
            Err(e) => {
                return self.hold_restart(t, intent.stage.as_deref(), &format!("{e:#}"), now_ms);
            }
        };
        let to = target_name(t, &old, intent.stage.as_deref());
        let mut checked = match self.check_restart(t, &old, intent.stage.as_deref()) {
            Ok(c) => c,
            Err(e) => return self.hold_restart(t, Some(&to), &format!("{e:#}"), now_ms),
        };
        let targets = std::mem::take(&mut checked.targets);
        if let Some(why) = self.reset_heads(t, targets, now_ms)? {
            return self.hold_restart(t, Some(&to), &why, now_ms);
        }
        let before = t.pipeline_file.clone();
        let after = self
            .data
            .ticket_dir(&t.id)
            .join(format!("pipeline.{}.toml", t.restarts.len() + 2));
        crate::store::atomic_write(&after, checked.text.as_bytes())?;
        let reset = t
            .restart
            .as_ref()
            .map(|i| i.reset.clone())
            .unwrap_or_default();
        let discarded = discard(t, &old, &checked);
        let setup_again = setup_changed(t, &old, &checked.new);
        if let Some(entry) = &checked.entry {
            for lane in &mut t.lanes {
                if let Some(at) = entry.lanes.get(&lane.name) {
                    lane.base_sha.clone_from(&at.base_sha);
                    lane.refreshed.clone_from(&at.refreshed);
                    lane.conflict.clone_from(&at.conflict);
                }
            }
        }
        remap(t, &old, &checked);
        t.pipeline_file.clone_from(&after);
        t.pipeline_fingerprint = Pipeline::fingerprint(&checked.text);
        t.restarts.push(Restart {
            at_ms: now_ms,
            from: checked.from.clone(),
            to: checked.to.clone(),
            before,
            after,
            discarded,
            reset,
            setup_again,
        });
        t.restart = None;
        t.state = TicketState::Active;
        t.state_by = None;
        if checked.ranged {
            self.record_entry(t, now_ms);
        }
        log::warn!(
            "ticket {} restarted at {} from {}",
            t.id,
            checked.to,
            checked.from
        );
        self.save_ticket(t, now_ms)
    }

    /// The ticket parked with why its restart cannot apply now; the
    /// intent stays for `dispatch restart` to carry on. `to` is `None`
    /// when no pipeline is readable to name the current stage from.
    fn hold_restart(
        &mut self,
        t: &mut Ticket,
        to: Option<&str>,
        why: &str,
        now_ms: u64,
    ) -> Result<()> {
        let reason = match to {
            Some(to) => format!("restart at {to} held: {why}"),
            None => format!("restart held: {why}"),
        };
        log::warn!("ticket {} parked: {reason}", t.id);
        t.state = TicketState::Parked { reason };
        self.save_ticket(t, now_ms)
    }

    /// Each branch whose head differs from its target's moved back to
    /// it, with `git reset --keep`: every one checked (on its branch, not
    /// mid-rebase, clean) before any moves, then each reset saved on the
    /// intent as it lands, so a restart cut short never moves a branch
    /// twice. `Some(why)` when the restart must hold.
    fn reset_heads(
        &mut self,
        t: &mut Ticket,
        targets: Vec<Target>,
        now_ms: u64,
    ) -> Result<Option<String>> {
        let done: Vec<String> = t
            .restart
            .as_ref()
            .map(|i| i.reset.iter().map(|h| h.key.clone()).collect())
            .unwrap_or_default();
        let nested: Vec<PathBuf> = targets
            .iter()
            .filter(|x| x.key != "root")
            .map(|x| x.dir.clone())
            .collect();
        let mut moves: Vec<(Target, String)> = Vec::new();
        for target in targets.into_iter().filter(|x| !done.contains(&x.key)) {
            let current = match self.git.branch_head(&target.dir, &target.branch) {
                Ok(Some(head)) => head,
                Ok(None) => {
                    return Ok(Some(format!(
                        "{} is not on its branch {}",
                        target.dir.display(),
                        target.branch
                    )));
                }
                Err(e) => return Ok(Some(format!("{e:#}"))),
            };
            if current == target.to {
                continue;
            }
            if matches!(self.git.rebase_in_progress(&target.dir), Ok(true)) {
                return Ok(Some(format!("{} is mid-rebase", target.dir.display())));
            }
            // A clean nested lane is untracked content in the ticket's
            // tree, so the tree is read with those lanes left out.
            let (tree, lanes) = if target.key == "root" {
                (Some(target.dir.as_path()), nested.clone())
            } else {
                (None, vec![target.dir.clone()])
            };
            let changed = crate::git::uncommitted(&*self.git, tree, &lanes)?;
            if !changed.is_empty() {
                let named: Vec<String> = changed.iter().map(|c| c.display().to_string()).collect();
                return Ok(Some(format!("changes in {}", named.join(", "))));
            }
            moves.push((target, current));
        }
        for (target, current) in moves {
            if let Err(e) = self.git.reset_branch(&target.dir, &target.to, &current) {
                return Ok(Some(format!("{e:#}")));
            }
            log::info!(
                "ticket {} {} reset from {current} to {}",
                t.id,
                target.key,
                target.to
            );
            if let Some(intent) = &mut t.restart {
                intent.reset.push(HeadReset {
                    key: target.key.clone(),
                    from: current,
                    to: target.to.clone(),
                });
            }
            self.save_ticket(t, now_ms)?;
        }
        Ok(None)
    }

    /// The branches a ranged restart to `to` moves, and the head each
    /// goes back to: the ticket's tree, and each lane with a repository
    /// of its own (a lane that is a path in the tree shares the tree's
    /// branch). `range` is the stages whose work is discarded and
    /// `entry` the heads recorded as the ticket entered the first of
    /// them. A reset is only needed when something after the target can
    /// have moved a branch, so for each branch:
    ///
    /// 1. the entry's head for it, when the entry has one;
    /// 2. else its current head, when no attempt can have moved it: no
    ///    agent, workflow or review attempt in the range in any state (a
    ///    failed or cancelled agent may have committed), and no rebaser
    ///    after the entry (any rebaser, without one);
    /// 3. else its current head, when it is on its branch, not
    ///    mid-rebase, has no commits beyond its base and a clean tree;
    /// 4. else the restart is refused.
    ///
    /// A gate-only attempt is assumed not to commit: its stage runs no
    /// agent, but its command gate runs the user's command in the lane
    /// tree, and nothing stops that command from committing. A commit it
    /// does make is kept and recorded in the entry the restart writes.
    /// `Err` is the refusal.
    fn reset_targets(
        &self,
        t: &Ticket,
        old: &Pipeline,
        to: &str,
        range: &[String],
        entry: Option<&StageEntry>,
    ) -> Result<Vec<Target>, String> {
        // Each branch with the base its own commits are counted from.
        let mut branches: Vec<(Target, String)> = Vec::new();
        if let Some(tree) = &t.tree {
            let recorded = t
                .lanes
                .iter()
                .filter(|l| !l.removed && old.lane(&l.name).is_some_and(|p| p.repo.is_none()))
                .find_map(|l| l.base_sha.clone());
            let base =
                recorded.unwrap_or_else(|| format!("{}/{}", old.project.remote, old.project.base));
            branches.push((
                Target {
                    key: "root".to_owned(),
                    dir: tree.clone(),
                    branch: tree_branch(t, None),
                    to: String::new(),
                },
                base,
            ));
        }
        for lane in t.lanes.iter().filter(|l| !l.removed) {
            if let Some(p) = old.lane(&lane.name).filter(|l| l.repo.is_some()) {
                let base = lane
                    .base_sha
                    .clone()
                    .unwrap_or_else(|| format!("{}/{}", old.lane_remote(p), old.lane_base(p)));
                branches.push((
                    Target {
                        key: lane.name.clone(),
                        dir: lane.worktree.clone(),
                        branch: lane.branch.clone(),
                        to: String::new(),
                    },
                    base,
                ));
            }
        }
        let may_have_moved = t.attempts.iter().any(|a| {
            let in_range = range.contains(&a.stage) && a.kind != AttemptKind::GateOnly;
            let rebaser = entry.map_or(a.stage == REFRESH || a.stage == RESOLUTION, |e| {
                after_entry(a, e)
            });
            in_range || rebaser
        });
        let nested: Vec<PathBuf> = branches
            .iter()
            .filter(|(x, _)| x.key != "root")
            .map(|(x, _)| x.dir.clone())
            .collect();
        let mut out = Vec::new();
        for (mut target, base) in branches {
            if let Some(head) = entry.and_then(|e| e.heads.get(&target.key)) {
                target.to.clone_from(head);
                out.push(target);
                continue;
            }
            let unrecorded = format!(
                "no head is recorded for {to} on this ticket and {}",
                target.key
            );
            let current = match self.git.branch_head(&target.dir, &target.branch) {
                Ok(Some(head)) => head,
                Ok(None) => {
                    return Err(format!(
                        "{unrecorded} is not on its branch {}; check it out, or restart at the current stage",
                        target.branch
                    ));
                }
                Err(e) => {
                    return Err(format!(
                        "{unrecorded}'s head cannot be read ({e:#}); restart at the current stage"
                    ));
                }
            };
            if may_have_moved {
                self.at_base(&target, &base, &nested, &unrecorded)?;
            }
            target.to = current;
            out.push(target);
        }
        Ok(out)
    }

    /// Whether a branch with no recorded head may stay where it is
    /// although something after the target can have moved it: not
    /// mid-rebase, no commits beyond `base`, and a clean tree. `nested`
    /// is the lanes with repositories of their own; `unrecorded` starts
    /// each refusal. `Err` is the refusal, naming which check failed.
    fn at_base(
        &self,
        target: &Target,
        base: &str,
        nested: &[PathBuf],
        unrecorded: &str,
    ) -> Result<(), String> {
        match self.git.rebase_in_progress(&target.dir) {
            Ok(false) => {}
            Ok(true) => {
                return Err(format!(
                    "{unrecorded} is mid-rebase; finish or abort the rebase, or restart at the current stage"
                ));
            }
            Err(e) => {
                return Err(format!(
                    "{unrecorded} cannot be read for a rebase in progress ({e:#}); restart at the current stage"
                ));
            }
        }
        match self.git.commits(&target.dir, base, "HEAD") {
            Ok(c) if c.is_empty() => {}
            Ok(_) => {
                return Err(format!(
                    "{unrecorded} has moved from its base; restart at the current stage, or close and retake"
                ));
            }
            Err(e) => {
                return Err(format!(
                    "{unrecorded}'s commits beyond its base cannot be read ({e:#}); restart at the current stage"
                ));
            }
        }
        // A clean nested lane is untracked content in the ticket's
        // tree, so the tree is read with those lanes left out.
        let (tree, lanes) = if target.key == "root" {
            (Some(target.dir.as_path()), nested.to_vec())
        } else {
            (None, vec![target.dir.clone()])
        };
        let changed =
            crate::git::uncommitted(&*self.git, tree, &lanes).map_err(|e| format!("{e:#}"))?;
        if !changed.is_empty() {
            let named: Vec<String> = changed.iter().map(|c| c.display().to_string()).collect();
            return Err(format!(
                "{unrecorded} has uncommitted changes in {}; clean the tree, or restart at the current stage",
                named.join(", ")
            ));
        }
        Ok(())
    }

    /// What each branch stands at as the ticket enters its current
    /// stage, pushed onto `entered`; the caller saves. Heads are read
    /// from the trees, which is local and cheap; one that cannot be read
    /// is left out with a log line and fails nothing now. A later ranged
    /// restart to this stage then keeps that branch where it is if
    /// nothing after the stage can have moved it or it is at its base,
    /// and is refused for it otherwise. A project that works in place
    /// has no branches to record.
    pub(crate) fn record_entry(&self, t: &mut Ticket, now_ms: u64) {
        let Some(tree) = t.tree.clone() else {
            return;
        };
        let names = crate::events::stage_names(t);
        let Some(stage) = names.get(t.stage).cloned() else {
            return;
        };
        let mut heads = BTreeMap::new();
        let mut read = |key: &str, dir: &std::path::Path| match self.git.head(dir) {
            Ok(head) => {
                heads.insert(key.to_owned(), head);
            }
            Err(e) => log::info!(
                "ticket {} entering {stage}: {key}'s head unreadable: {e:#}",
                t.id
            ),
        };
        read("root", &tree);
        let mut lanes = BTreeMap::new();
        for lane in t.lanes.iter().filter(|l| !l.removed) {
            read(&lane.name, &lane.worktree);
            lanes.insert(
                lane.name.clone(),
                LaneAtEntry {
                    base_sha: lane.base_sha.clone(),
                    refreshed: lane.refreshed.clone(),
                    conflict: lane.conflict.clone(),
                },
            );
        }
        t.entered.push(StageEntry {
            stage,
            at_ms: now_ms,
            heads,
            lanes,
        });
    }
}

/// Where `to` stands in each copy: its index in the ticket's copy (the
/// stage after it when only the live file has it), its index in the
/// live file, and whether only the live file has it. Refused when it
/// comes after the current stage `from` (or, for a live-only `to`, when
/// the live file lacks `from` and the stage after `to` is later than
/// it), or a stage before it in the live file was never run.
fn locate(
    t: &Ticket,
    old: &Pipeline,
    new: &Pipeline,
    from: &str,
    to: &str,
) -> Result<(usize, usize, bool)> {
    let in_old = old.stages.iter().position(|s| s.name == to);
    let Some(target_new) = new.stages.iter().position(|s| s.name == to) else {
        if in_old.is_none() {
            bail!("{to} is not a stage of ticket {}'s pipeline", t.id);
        }
        bail!("the live pipeline has no stage {to}");
    };
    // When the live file renamed or dropped the current stage, the
    // follower of a live-only `to` can land after it without `to` being
    // later; say what is actually missing.
    let after = || {
        if new.stages.iter().any(|s| s.name == from) {
            format!("{to} comes after the current stage {from}; a restart only goes back")
        } else {
            format!("the live pipeline has no stage {from}, the ticket's current stage")
        }
    };
    // A stage only the live file has stands where the first live
    // stage after it that the ticket ran stands.
    let live_only = in_old.is_none();
    let target_old = match in_old {
        Some(i) => i,
        None => new.stages[target_new + 1..]
            .iter()
            .find_map(|s| old.stages.iter().position(|o| o.name == s.name))
            .ok_or_else(|| anyhow::anyhow!(after()))?,
    };
    if target_old > t.stage {
        bail!("{}", after());
    }
    let ran: Vec<&str> = old.stages[..target_old]
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    if let Some(added) = new.stages[..target_new]
        .iter()
        .find(|s| !ran.contains(&s.name.as_str()))
    {
        // A restart at the added stage is ranged, so it is pointed
        // at only for a ticket that can take one.
        let advice = if can_range(t, old) {
            format!("; restart it at {} to run it", added.name)
        } else {
            String::new()
        };
        bail!(
            "the live pipeline adds {} before {to}, which this ticket never ran{advice}",
            added.name
        );
    }
    Ok((target_old, target_new, live_only))
}

/// What a restart compares a lane by: where it lives and what it
/// branches from.
fn lane_shape(l: &Lane) -> (&PathBuf, &Option<String>, &Option<String>, &Option<String>) {
    (&l.path, &l.repo, &l.base, &l.remote)
}

/// The stage a restart goes to, by name: the one named, else the
/// current one.
fn target_name(t: &Ticket, p: &Pipeline, stage: Option<&str>) -> String {
    stage.map_or_else(
        || {
            p.stages
                .get(t.stage)
                .map_or_else(String::new, |s| s.name.clone())
        },
        str::to_owned,
    )
}

/// The newest entry recorded for `stage`.
fn latest_entry<'t>(t: &'t Ticket, stage: &str) -> Option<&'t StageEntry> {
    t.entered.iter().rev().find(|e| e.stage == stage)
}

/// Whether a ticket can take a ranged restart: its project cuts
/// worktrees, so there are branches to reset, and the branches are its
/// own, not a pull request's.
fn can_range(t: &Ticket, old: &Pipeline) -> bool {
    old.cuts_worktrees() && !t.source.is_pull_request()
}

/// A rebaser or a resolution review started after the entry: it worked
/// on a branch a reset to the entry moves away from.
fn after_entry(a: &Attempt, e: &StageEntry) -> bool {
    (a.stage == REFRESH || a.stage == RESOLUTION) && a.started_ms >= e.at_ms
}

/// The completed attempts in the range cancelled, so nothing reads them
/// as done; their decisions and the range's cancelled; `lanes` asked
/// again when it is in the range; and on a plain restart the attempts
/// whose checks may be run again under the new copy flagged so their
/// question offers `check`. Returns the attempts discarded.
fn discard(t: &mut Ticket, old: &Pipeline, c: &Checked) -> Vec<(String, u32)> {
    let range = &c.range;
    let reason = format!("{DISCARDED_BY}{}", c.to);
    let mut discarded = Vec::new();
    for a in &mut t.attempts {
        let rebased = c.entry.as_ref().is_some_and(|e| after_entry(a, e));
        if a.state == AttemptState::Complete && (range.contains(&a.stage) || rebased) {
            a.state = AttemptState::Cancelled {
                reason: reason.clone(),
            };
            discarded.push((a.stage.clone(), a.n));
        }
    }
    if !c.ranged {
        let mut contexts: Vec<String> = t
            .attempts
            .iter()
            .filter(|a| a.stage == c.from)
            .map(|a| a.context.clone())
            .collect();
        contexts.sort();
        contexts.dedup();
        for ctx in contexts {
            let Some(a) = t
                .attempts
                .iter_mut()
                .filter(|a| a.stage == c.from && a.context == ctx)
                .max_by_key(|a| a.n)
            else {
                continue;
            };
            if !matches!(a.state, AttemptState::Cancelled { .. }) {
                continue;
            }
            let was_complete = discarded.contains(&(a.stage.clone(), a.n));
            // A finished review's fold and summary are done; checks that
            // pass would rewrite its commits again, so it gets no `check`.
            a.failed_at_checks |= match a.kind {
                AttemptKind::Agent => a.gate.is_some(),
                AttemptKind::Review => !was_complete && a.gate.is_some() && a.rewrite.is_none(),
                AttemptKind::Workflow | AttemptKind::GateOnly => false,
            };
        }
    }
    for d in &mut t.decisions {
        if d.state == DecisionState::Cancelled {
            continue;
        }
        let about_discarded = d.attempt.as_ref().is_some_and(|k| discarded.contains(k));
        if about_discarded || range.contains(&d.stage) {
            d.state = DecisionState::Cancelled;
        }
    }
    let lanes_again = old.stages.iter().any(|s| {
        range.contains(&s.name)
            && matches!(&s.gate, Some(Gate::Human { decision, .. }) if decision == "lanes")
    });
    if lanes_again {
        for lane in &mut t.lanes {
            lane.chosen = true;
        }
    }
    t.rework
        .retain(|key, _| !range.iter().any(|s| key.starts_with(&rework_key(s, ""))));
    discarded
}

/// The lanes whose `setup` differs between the copies, marked to run it
/// again before their next agent or checks.
fn setup_changed(t: &mut Ticket, old: &Pipeline, new: &Pipeline) -> Vec<String> {
    let mut again = Vec::new();
    for lane in t.lanes.iter_mut().filter(|l| !l.removed) {
        let before = old.lane(&lane.name).map(|l| &l.setup);
        let after = new.lane(&lane.name).map(|l| &l.setup);
        if before != after {
            lane.setup_done = false;
            again.push(lane.name.clone());
        }
    }
    again
}

/// Every stage index on the ticket moved into the new copy by name: the
/// current stage, the stage last brought up (kept on a plain restart so
/// no bring-up runs; cleared on a ranged one so the reset branches are
/// brought up on re-entry), and each recorded conflict's stage. A
/// conflict whose stage is gone from the new copy, or in the discarded
/// range, is dropped, so no resolution review is started on a stale
/// index.
fn remap(t: &mut Ticket, old: &Pipeline, c: &Checked) {
    let names: Vec<&str> = old.stages.iter().map(|s| s.name.as_str()).collect();
    let moved = |i: usize, drop_range: bool| -> Option<usize> {
        if drop_range && c.ranged && i >= c.target_old {
            return None;
        }
        let name = names.get(i)?;
        c.new.stages.iter().position(|s| s.name == *name)
    };
    let fix = |conflict: &mut Option<crate::ticket::RefreshConflict>| {
        if let Some(k) = conflict {
            match moved(k.stage, true) {
                Some(i) => k.stage = i,
                None => *conflict = None,
            }
        }
    };
    for lane in &mut t.lanes {
        fix(&mut lane.conflict);
        if let Some(r) = &mut lane.refreshed {
            fix(&mut r.conflict);
        }
    }
    for entry in &mut t.entered {
        for lane in entry.lanes.values_mut() {
            fix(&mut lane.conflict);
            if let Some(r) = &mut lane.refreshed {
                fix(&mut r.conflict);
            }
        }
    }
    t.refreshed_stage = if c.ranged {
        None
    } else {
        t.refreshed_stage.and_then(|i| moved(i, false))
    };
    t.stage = c.target_new;
}
