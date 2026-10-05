//! Restarting a ticket: put it at its stage, or an earlier one, under a
//! fresh copy of the project's live pipeline. The restart rides on a
//! park, so everything running is read back as gone before anything
//! moves; then the branches go back to the heads recorded as the ticket
//! entered the stage, the later work is discarded, and the stage asks
//! before any agent of it runs again.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};

use crate::pipeline::{Gate, Lane, Pipeline};
use crate::scheduler::{REFRESH, RESOLUTION, Runner, rework_key, tree_branch};
use crate::ticket::{
    AttemptKind, AttemptState, DecisionState, HeadReset, LaneAtEntry, Restart, RestartIntent,
    StageEntry, Ticket, TicketState,
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
    /// `to`'s index in the ticket's copy.
    target_old: usize,
    /// `to`'s index in the live file.
    target_new: usize,
    /// `to` is earlier than the current stage: later work is discarded
    /// and the branches reset.
    ranged: bool,
}

/// A branch a ranged restart moves: its key in `StageEntry::heads`, its
/// tree, the branch checked out there, and the head it goes back to.
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
        let Some(target_old) = old.stages.iter().position(|s| s.name == to) else {
            bail!("{to} is not a stage of ticket {}'s pipeline", t.id);
        };
        let Some(target_new) = new.stages.iter().position(|s| s.name == to) else {
            bail!("the live pipeline has no stage {to}");
        };
        if target_old > t.stage {
            bail!("{to} comes after the current stage {from}; a restart only goes back");
        }
        let ran: Vec<&str> = old.stages[..target_old]
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        if let Some(added) = new.stages[..target_new]
            .iter()
            .find(|s| !ran.contains(&s.name.as_str()))
        {
            bail!(
                "the live pipeline adds {} before {to}, which this ticket never ran",
                added.name
            );
        }
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
        let ranged = target_old < t.stage;
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
            let missing = format!(
                "no head is recorded for {to} on this ticket; restart at the current stage, or close and retake"
            );
            let Some(entry) = latest_entry(t, &to) else {
                bail!("{missing}");
            };
            if targets(t, old, entry).is_none() {
                bail!("{missing}");
            }
        }
        Ok(Checked {
            text,
            new,
            from,
            to,
            target_old,
            target_new,
            ranged,
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
        let checked = match self.check_restart(t, &old, intent.stage.as_deref()) {
            Ok(c) => c,
            Err(e) => return self.hold_restart(t, Some(&to), &format!("{e:#}"), now_ms),
        };
        let entry = if checked.ranged {
            latest_entry(t, &checked.to).cloned()
        } else {
            None
        };
        if let Some(entry) = &entry
            && let Some(why) = self.reset_heads(t, &old, entry, now_ms)?
        {
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
        let discarded = discard(t, &old, &checked, entry.as_ref());
        let setup_again = setup_changed(t, &old, &checked.new);
        if let Some(entry) = &entry {
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

    /// Each branch whose head differs from the entry's moved back to
    /// it, with `git reset --keep`: every one checked (on its branch, not
    /// mid-rebase, clean) before any moves, then each reset saved on the
    /// intent as it lands, so a restart cut short never moves a branch
    /// twice. `Some(why)` when the restart must hold.
    fn reset_heads(
        &mut self,
        t: &mut Ticket,
        old: &Pipeline,
        entry: &StageEntry,
        now_ms: u64,
    ) -> Result<Option<String>> {
        let Some(targets) = targets(t, old, entry) else {
            return Ok(Some(format!("no head is recorded for {}", entry.stage)));
        };
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

    /// What each branch stands at as the ticket enters its current
    /// stage, pushed onto `entered`; the caller saves. Heads are read
    /// from the trees, which is local and cheap; one that cannot be read
    /// is left out with a log line, which refuses a later ranged restart
    /// to this stage and fails nothing now. A project that works in
    /// place has no branches to record.
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

/// The branches a ranged restart moves, with their entry heads: the
/// ticket's tree, and each lane with a repository of its own. A lane
/// that is a path in the tree shares the tree's branch. `None` when the
/// entry lacks one of them.
fn targets(t: &Ticket, p: &Pipeline, entry: &StageEntry) -> Option<Vec<Target>> {
    let mut out = Vec::new();
    if let Some(tree) = &t.tree {
        out.push(Target {
            key: "root".to_owned(),
            dir: tree.clone(),
            branch: tree_branch(t, None),
            to: entry.heads.get("root")?.clone(),
        });
    }
    for lane in t.lanes.iter().filter(|l| !l.removed) {
        if p.lane(&lane.name).is_some_and(|l| l.repo.is_some()) {
            out.push(Target {
                key: lane.name.clone(),
                dir: lane.worktree.clone(),
                branch: lane.branch.clone(),
                to: entry.heads.get(&lane.name)?.clone(),
            });
        }
    }
    Some(out)
}

/// The stages whose work the restart discards, by name: from the target
/// on in the ticket's copy for a ranged restart, the current stage alone
/// for a plain one.
fn range(old: &Pipeline, c: &Checked) -> Vec<String> {
    if c.ranged {
        old.stages[c.target_old..]
            .iter()
            .map(|s| s.name.clone())
            .collect()
    } else {
        vec![c.from.clone()]
    }
}

/// The completed attempts in the range cancelled, so nothing reads them
/// as done; their decisions and the range's cancelled; `lanes` asked
/// again when it is in the range; and on a plain restart the attempts
/// whose checks may be run again under the new copy flagged so their
/// question offers `check`. Returns the attempts discarded.
fn discard(
    t: &mut Ticket,
    old: &Pipeline,
    c: &Checked,
    entry: Option<&StageEntry>,
) -> Vec<(String, u32)> {
    let range = range(old, c);
    let reason = format!("{DISCARDED_BY}{}", c.to);
    let mut discarded = Vec::new();
    for a in &mut t.attempts {
        // A rebaser or a resolution review after the entry worked on a
        // branch the reset moved away from.
        let after_entry = entry.is_some_and(|e| {
            (a.stage == REFRESH || a.stage == RESOLUTION) && a.started_ms >= e.at_ms
        });
        if a.state == AttemptState::Complete && (range.contains(&a.stage) || after_entry) {
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
