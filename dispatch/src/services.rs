//! A stage's resources and services. A ticket takes a hold on each
//! `[[resources]]` entry its stage `needs` on entering a run of
//! consecutive stages that need it, and drops it the pass after it
//! leaves the run, once the services started inside it are confirmed
//! stopped; a later run takes it again. A stage's
//! `services` are the lanes it serves: the lane's `before` as a child of
//! the runner, a free port, a Switchboard service session launched with
//! the port in its environment, and the readiness probe. Both live on
//! the ticket record; the scheduler steps them.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use switchboard_control::{self as wire, Body, Made, Reply};

use crate::pipeline::{Pipeline, Process, ServeRef, ServeSpec, Stage};
use crate::scheduler::{Ask, NO_SUCH_SESSION, Runner, SocketDown, confine_for, env_for};
use crate::ticket::{
    Attempt, AttemptState, Decision, DecisionKind, DecisionState, Forgotten, GateRun, Hold,
    LaneRecord, ProjectState, STUCK, ServiceRecord, ServiceState, Ticket, TicketState,
};

/// How long a service may take to read as stopped (its `before` exited,
/// its session gone, its port free) before the user is asked: well
/// past the seconds a setup command or a dev server's exit takes.
pub const STOP_LIMIT_MS: u64 = 120_000;

/// The decision a service that could not be brought up raises: `retry`
/// or `park`.
pub const SERVICE: &str = "service";

impl Runner {
    // --- holds

    /// Each resource the stage `needs` that the ticket does not hold yet,
    /// taken when fewer than its `count` of the project's other tickets
    /// hold it, as their records say now (read under the writer lock;
    /// one runner writes them). True while the stage must wait: a
    /// resource is held elsewhere, or the ticket parked on a lane named
    /// as a need.
    pub(crate) fn take_holds(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<bool> {
        for need in &stage.needs {
            if t.holds.iter().any(|h| &h.resource == need) {
                continue;
            }
            let Some(resource) = p.resource(need) else {
                self.park(
                    t,
                    ps,
                    &format!(
                        "stage {} needs lane {need} (an in-place hold), which is not built in this slice",
                        stage.name
                    ),
                    now_ms,
                )?;
                return Ok(true);
            };
            let holders = holders_of(&self.tickets()?, t, need);
            if holders.len() < resource.count as usize {
                t.holds.push(Hold {
                    resource: need.clone(),
                    stage: stage.name.clone(),
                    taken_ms: now_ms,
                });
                self.held_back.remove(&t.id);
                log::info!("ticket {} holds {need} from {}", t.id, stage.name);
                if let Some(back) = undo_range(t, p, need, now_ms) {
                    log::info!(
                        "ticket {} back to {}: {need} was let go since it ran",
                        t.id,
                        p.stages[back].name
                    );
                    t.stage = back;
                    self.save_ticket(t, now_ms)?;
                    return Ok(true);
                }
                self.save_ticket(t, now_ms)?;
                continue;
            }
            // One line when the wait starts, not one a pass.
            if self.held_back.get(&t.id) != Some(need) {
                log::info!(
                    "ticket {} at {} waits for {need}, held by {}",
                    t.id,
                    stage.name,
                    holders.join(", ")
                );
                self.held_back.insert(t.id.clone(), need.clone());
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Every service whose range no longer holds the current stage is
    /// stopped, and once each reads as stopped, every hold the current
    /// stage does not need is dropped. Run on every pass, so any way the
    /// stage changes (an advance, a send-back, the pipeline's end) lets
    /// go one pass later. True while a stop is still going, so nothing
    /// else of the ticket moves and the hold stays.
    pub(crate) fn release_left(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<bool> {
        self.forget_secrets(t, Some(p), now_ms)?;
        let leaving: Vec<usize> = t
            .services
            .iter()
            .enumerate()
            .filter(|(_, s)| s.state != ServiceState::Stopped && !in_range(p, s, t.stage))
            .map(|(i, _)| i)
            .collect();
        let mut stopped = true;
        for &i in &leaving {
            stopped &= self.stop_service(t, ps, i, now_ms)?;
            if !t.active() {
                return Ok(true);
            }
        }
        if !stopped {
            return Ok(true);
        }
        let needs: &[String] = p.stages.get(t.stage).map_or(&[], |s| s.needs.as_slice());
        let (mut kept, dropped): (Vec<Hold>, Vec<Hold>) =
            t.holds.drain(..).partition(|h| needs.contains(&h.resource));
        // A hold kept across a send-back into another run of its
        // resource is now held for that run: the waiting text names the
        // stage it is held from.
        let mut rewritten = false;
        for h in &mut kept {
            let taken_in = p
                .stages
                .iter()
                .position(|s| s.name == h.stage)
                .and_then(|i| p.hold_run_at(&h.resource, i));
            // `kept` is empty unless `needs` came from the current stage.
            if taken_in != p.hold_run_at(&h.resource, t.stage) {
                h.stage.clone_from(&p.stages[t.stage].name);
                rewritten = true;
            }
        }
        t.holds = kept;
        if dropped.is_empty() {
            if rewritten {
                self.save_ticket(t, now_ms)?;
            }
            return Ok(false);
        }
        let by_hand: Vec<String> = leaving
            .iter()
            .filter_map(|&i| t.services[i].released.clone())
            .collect();
        for h in &dropped {
            if by_hand.is_empty() {
                log::info!("ticket {} released {}", t.id, h.resource);
            } else {
                log::info!(
                    "ticket {} released {}: released by hand: {}",
                    t.id,
                    h.resource,
                    by_hand.join("; ")
                );
            }
        }
        self.save_ticket(t, now_ms)?;
        Ok(false)
    }

    /// Every secret artifact whose reason to exist has ended deleted,
    /// and the deletion recorded: its attempt failed, was cancelled or
    /// was replaced, the ticket left the run its stage's hold covers,
    /// or the ticket is parking or closing. An attempt still starting
    /// or running is never swept, since its command may yet write the
    /// file. The file goes first and the record after, so a crash in
    /// between is recorded on the next pass. Without the ticket's
    /// pipeline the hold's run is not judged. True when anything was
    /// recorded.
    pub(crate) fn forget_secrets(
        &mut self,
        t: &mut Ticket,
        p: Option<&Pipeline>,
        now_ms: u64,
    ) -> Result<bool> {
        if !t
            .attempts
            .iter()
            .any(|a| a.secret.iter().any(|n| !a.forgotten.contains_key(n)))
        {
            return Ok(false);
        }
        let mut gone: Vec<(usize, String, String)> = Vec::new();
        for (i, a) in t.attempts.iter().enumerate() {
            if a.is_open() {
                continue;
            }
            let Some(why) = Self::forget_reason(t, p, a) else {
                continue;
            };
            for name in a.secret.iter().filter(|n| !a.forgotten.contains_key(*n)) {
                if let Some(path) = a.artifacts.get(name) {
                    match std::fs::remove_file(path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => {
                            log::warn!(
                                "ticket {} could not delete secret {name} at {}: {}",
                                t.id,
                                path.display(),
                                e.kind()
                            );
                            continue;
                        }
                    }
                }
                gone.push((i, name.clone(), why.clone()));
            }
        }
        if gone.is_empty() {
            return Ok(false);
        }
        for (i, name, why) in gone {
            log::info!("ticket {} deleted secret {name}: {why}", t.id);
            t.attempts[i]
                .forgotten
                .insert(name, Forgotten { at_ms: now_ms, why });
        }
        self.save_ticket(t, now_ms)?;
        Ok(true)
    }

    /// Why a stopped attempt's secret artifacts are to be deleted now;
    /// `None` while they are still the stage's to use.
    fn forget_reason(t: &Ticket, p: Option<&Pipeline>, a: &Attempt) -> Option<String> {
        match a.state {
            AttemptState::Failed { .. } => return Some("attempt failed".to_owned()),
            AttemptState::Cancelled { .. } => return Some("attempt cancelled".to_owned()),
            AttemptState::Starting | AttemptState::Running | AttemptState::Complete => {}
        }
        let newer = t.attempts.iter().any(|b| {
            b.stage == a.stage
                && b.context == a.context
                && b.n > a.n
                && b.state == AttemptState::Complete
        });
        if newer {
            return Some("attempt replaced".to_owned());
        }
        if let Some(p) = p {
            let index = p.stages.iter().position(|s| s.name == a.stage);
            let held = index
                .and_then(|i| p.needs_range(i))
                .is_some_and(|(first, last)| (first..=last).contains(&t.stage));
            if !held {
                let resource = index
                    .and_then(|i| p.stages[i].needs.iter().find(|n| p.resource(n).is_some()))
                    .map_or("the hold", String::as_str);
                return Some(format!("{resource} released"));
            }
        }
        match t.state {
            TicketState::Parking { .. } | TicketState::Parked { .. } => Some("parked".to_owned()),
            TicketState::Closing { .. } | TicketState::Closed { .. } => Some("closed".to_owned()),
            TicketState::Active => None,
        }
    }

    // --- services

    /// The stage's services, a lane after the lanes its `serve.env`
    /// names, brought one step further each pass. A lane naming another
    /// starts once that one answers, and is restarted when that one is
    /// served again. A lane the ticket did not choose is not served, and
    /// one naming it fails. True once every served lane answers; until
    /// then no agent of the stage launches. A lane that could not be
    /// served is stopped, then asked about once (`retry` or `park`). A
    /// lane's processes come up together and go together: one `before`,
    /// then every launch, and one process failing stops the others.
    pub(crate) fn ensure_services(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        now_ms: u64,
    ) -> Result<bool> {
        let mut ready = true;
        let mut failed: Vec<String> = Vec::new();
        // Validation refuses a cycle. A pipeline stored on an older ticket
        // predates lane references and so cannot hold one; if what is
        // stored ever changes, this fallback would serve in list order.
        let order = p
            .serve_order(stage)
            .unwrap_or_else(|_| stage.services.clone());
        for lane in &order {
            let Some(record) = t
                .lanes
                .iter()
                .find(|l| &l.name == lane && l.chosen)
                .cloned()
            else {
                continue;
            };
            let Some(spec) = p.lane(lane).and_then(|l| l.serve.clone()) else {
                continue;
            };
            let mut records = Vec::new();
            for process in spec.processes().iter() {
                records.push(self.service_record(
                    t,
                    p,
                    &stage.name,
                    lane,
                    process.name.as_deref(),
                    now_ms,
                )?);
            }
            self.mark_stale_pointer(t, p, &records, now_ms)?;
            // Marked by a send-back: stopped before it counts, since a
            // record being stopped still reads `Ready` until the stop is
            // confirmed, and an agent must not start against it. Once
            // `Stopped`, the next pass makes a fresh record; the lane's
            // other records wait until none is left to stop.
            let marked: Vec<usize> = records
                .iter()
                .copied()
                .filter(|&i| {
                    t.services[i].stopping_ms.is_some()
                        && matches!(
                            t.services[i].state,
                            ServiceState::Before | ServiceState::Starting | ServiceState::Ready
                        )
                })
                .collect();
            if !marked.is_empty() {
                ready = false;
                for i in marked {
                    self.stop_service(t, ps, i, now_ms)?;
                    if !t.active() {
                        return Ok(false);
                    }
                }
                continue;
            }
            if !self.named_lanes_up(t, p, &stage.name, &spec, records[0], now_ms)? {
                ready = false;
                continue;
            }
            if !records.iter().any(|&i| is_failed(&t.services[i])) {
                let cx = LaneCx {
                    p,
                    stage,
                    record: &record,
                };
                let processes = spec.processes();
                self.advance_lane(t, ps, &cx, processes.iter().zip(&records), now_ms)?;
                if !t.active() {
                    return Ok(false);
                }
            }
            if records.iter().any(|&i| is_failed(&t.services[i])) {
                ready = false;
                failed.extend(self.fail_lane(t, ps, &records, now_ms)?);
                if !t.active() {
                    return Ok(false);
                }
                continue;
            }
            if records
                .iter()
                .any(|&i| t.services[i].state != ServiceState::Ready)
            {
                ready = false;
            }
        }
        if !failed.is_empty() {
            self.ask_service(t, ps, &stage.name, &failed, now_ms)?;
        }
        Ok(ready)
    }

    /// The one `service` question of a stage: the lanes that could not
    /// be served, with what to do about it.
    fn ask_service(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        stage: &str,
        failed: &[String],
        now_ms: u64,
    ) -> Result<()> {
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage,
                name: SERVICE,
                kind: DecisionKind::Permission,
                question: format!(
                    "{stage}: could not serve {}. Retry starts each again (its before, a new port), park stops the ticket.",
                    failed.join("; ")
                ),
                options: &["retry", "park"],
                recommendation: None,
                attempt: None,
            },
            now_ms,
        )?;
        Ok(())
    }

    /// Whether the lanes a lane's `serve.env` names let it start: a lane
    /// naming another waits for it, its `before` included, and one
    /// naming a lane the ticket did not cut fails, since nothing will
    /// ever serve that. True when the lane may go on: it is past
    /// `Before`, or it was just failed here for an unserved name (the
    /// caller then sees the failure and stops the lane).
    fn named_lanes_up(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        stage: &str,
        spec: &ServeSpec,
        first: usize,
        now_ms: u64,
    ) -> Result<bool> {
        if t.services[first].state != ServiceState::Before {
            return Ok(true);
        }
        if let Some((missing, named)) = unserved_ref(t, spec) {
            let reason = format!(
                "{} is not served (the ticket did not choose {named})",
                missing.token()
            );
            self.fail_service(t, first, &reason, now_ms)?;
            return Ok(true);
        }
        Ok(named_ready(t, p, stage, spec))
    }

    /// Any of a lane's records launched against an address that is
    /// gone marks every one of the lane's records (`Before` included) to
    /// be stopped and started again the way a send-back marks them, since
    /// a lane's processes come up together.
    fn mark_stale_pointer(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        records: &[usize],
        now_ms: u64,
    ) -> Result<()> {
        if !records.iter().any(|&i| stale_pointer(t, p, i)) {
            return Ok(());
        }
        let mut marked = false;
        for &i in records {
            let rec = &mut t.services[i];
            if rec.stopping_ms.is_none()
                && matches!(
                    rec.state,
                    ServiceState::Before | ServiceState::Starting | ServiceState::Ready
                )
            {
                log::info!(
                    "ticket {} {} restarts {}: a lane it names was served again",
                    t.id,
                    rec.stage,
                    rec.label()
                );
                rec.stopping_ms = Some(now_ms);
                marked = true;
            }
        }
        if marked {
            self.save_ticket(t, now_ms)?;
        }
        Ok(())
    }

    /// Each of a lane's records one step further, in the lane's order:
    /// the first runs the lane's `before`, the rest launch once it has.
    fn advance_lane<'a>(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        cx: &LaneCx,
        work: impl Iterator<Item = (&'a Process, &'a usize)>,
        now_ms: u64,
    ) -> Result<()> {
        let mut first = None;
        for (process, &i) in work {
            let first = *first.get_or_insert(i);
            match t.services[i].state.clone() {
                ServiceState::Before if first == i => {
                    self.service_before(t, ps, cx, process, i, now_ms)?;
                }
                // After the first process's `before`, so every process
                // starts on what it set up.
                ServiceState::Before => {
                    if matches!(
                        t.services[first].state,
                        ServiceState::Starting | ServiceState::Ready
                    ) {
                        self.launch_service(t, ps, cx.p, process, i, now_ms)?;
                    }
                }
                ServiceState::Starting => {
                    self.service_starting(t, ps, cx.p, process, i, now_ms)?;
                }
                // Only a process of a list is read again once ready.
                ServiceState::Ready if process.name.is_some() => {
                    self.service_running(t, i, now_ms)?;
                }
                ServiceState::Ready | ServiceState::Failed { .. } | ServiceState::Stopped => {}
            }
            if !t.active() {
                return Ok(());
            }
        }
        Ok(())
    }

    /// A lane one of whose records failed: the others fail with it and
    /// every record is stopped. The failures to ask about, once every
    /// record is confirmed gone, so a `retry` never starts a second one
    /// beside it; empty until then.
    fn fail_lane(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        records: &[usize],
        now_ms: u64,
    ) -> Result<Vec<String>> {
        let Some(&first) = records.iter().find(|&&i| is_failed(&t.services[i])) else {
            return Ok(Vec::new());
        };
        let with = format!("stopped with {}", t.services[first].label());
        for &i in records {
            if !is_failed(&t.services[i]) {
                self.fail_service(t, i, &with, now_ms)?;
            }
        }
        let mut gone = true;
        let mut reasons = Vec::new();
        for &i in records {
            let ServiceState::Failed { reason } = t.services[i].state.clone() else {
                continue;
            };
            let end = ServiceState::Failed {
                reason: reason.clone(),
            };
            gone &= self.stop_service_into(t, ps, i, end, now_ms)?;
            if !t.active() {
                return Ok(Vec::new());
            }
            reasons.push(format!("{}: {reason}", t.services[i].label()));
        }
        Ok(if gone { reasons } else { Vec::new() })
    }

    /// A `retry` answer to a `service` question: each failed service of
    /// the current stage is stopped, and each that reads as stopped is
    /// written `Stopped`, so the next pass makes a fresh record (`before`
    /// again, a new port). One whose stop is not confirmed yet stays
    /// failed and is stopped on, then asked about, so a retry never
    /// starts a lane beside a copy still alive.
    pub(crate) fn retry_services(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        now_ms: u64,
    ) -> Result<()> {
        let Some(stage) = p.stages.get(t.stage) else {
            return Ok(());
        };
        for i in 0..t.services.len() {
            let rec = &t.services[i];
            if rec.stage == stage.name && matches!(rec.state, ServiceState::Failed { .. }) {
                self.stop_service(t, ps, i, now_ms)?;
                if !t.active() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// The index of the stage's live record for `lane` and `process`
    /// (`None` for the single table), made in `Before` when there is
    /// none.
    fn service_record(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        stage: &str,
        lane: &str,
        process: Option<&str>,
        now_ms: u64,
    ) -> Result<usize> {
        let same = |s: &ServiceRecord| s.is_for(stage, lane, process);
        if let Some(i) = t
            .services
            .iter()
            .rposition(|s| same(s) && s.state != ServiceState::Stopped)
        {
            return Ok(i);
        }
        let n = t
            .services
            .iter()
            .filter(|s| same(s))
            .map(|s| s.n)
            .max()
            .unwrap_or(0)
            + 1;
        let until = p
            .stages
            .get(p.services_until(t.stage))
            .map_or_else(|| stage.to_owned(), |s| s.name.clone());
        t.services.push(ServiceRecord {
            lane: lane.to_owned(),
            n,
            stage: stage.to_owned(),
            until,
            before: None,
            port: None,
            url: None,
            op: None,
            session: None,
            state: ServiceState::Before,
            started_ms: now_ms,
            ready_ms: None,
            stopping_ms: None,
            stuck_on: None,
            released: None,
            pointed_at: BTreeMap::new(),
            process: process.map(str::to_owned),
            group: None,
        });
        self.save_ticket(t, now_ms)?;
        Ok(t.services.len() - 1)
    }

    /// The lane's `before`, started, watched, and once it exits 0 (or
    /// with none) the first process's launch.
    fn service_before(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        cx: &LaneCx,
        process: &Process,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let LaneCx {
            p,
            stage,
            record: lane,
        } = *cx;
        if let Some(argv) = stage.before.get(&lane.name) {
            let key = before_key(t, &t.services[i]);
            let exit = match t.services[i].before.as_ref().map(|b| b.exit) {
                None => return self.start_before(t, p, lane, argv, &key, i, now_ms),
                Some(Some(code)) => code,
                // The real child is gone after the read that sees its
                // exit, so the exit is written in the same save as what
                // follows from it.
                Some(None) => match self.git.poll_check(&key) {
                    None => return Ok(()),
                    Some(Ok(code)) => {
                        if let Some(b) = &mut t.services[i].before {
                            b.exit = Some(code);
                        }
                        code
                    }
                    // Lost with a runner that restarted. A `before` must
                    // be safe to run again (docs/dispatch.md), so it is
                    // started again.
                    Some(Err(e)) => {
                        log::warn!(
                            "ticket {} {} before for {} lost ({e:#}); starting again",
                            t.id,
                            stage.name,
                            lane.name
                        );
                        return self.start_before(t, p, lane, argv, &key, i, now_ms);
                    }
                },
            };
            if exit != 0 {
                let log = t.services[i]
                    .before
                    .as_ref()
                    .map(|b| b.log.display().to_string())
                    .unwrap_or_default();
                return self.fail_service(
                    t,
                    i,
                    &format!("before exited {exit}; see {log}"),
                    now_ms,
                );
            }
        }
        self.launch_service(t, ps, p, process, i, now_ms)
    }

    /// A free port, when the process has an address or names `{port}`,
    /// and the launch.
    fn launch_service(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        process: &Process,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let port = if process.wants_port() {
            let Some(port) = self.free_port(t, p)? else {
                let range = p
                    .policy
                    .ports
                    .map(|[lo, hi]| format!("{lo}-{hi}"))
                    .unwrap_or_default();
                return self.fail_service(t, i, &format!("no free port in {range}"), now_ms);
            };
            Some(port)
        } else {
            None
        };
        let rec = &mut t.services[i];
        rec.port = port;
        rec.url = port.and_then(|port| {
            process
                .url
                .as_ref()
                .map(|u| u.replace("{port}", &port.to_string()))
        });
        rec.state = ServiceState::Starting;
        rec.started_ms = now_ms;
        let on = port
            .map(|port| format!(" on port {port}"))
            .unwrap_or_default();
        log::info!("ticket {} {} serves {}{on}", t.id, rec.stage, rec.label());
        self.save_ticket(t, now_ms)?;
        self.send_service(t, ps, p, process, i, now_ms)
    }

    /// The lane's `before` started as a child of the runner, its log in
    /// the ticket's directory under the record's number.
    #[allow(clippy::too_many_arguments)]
    fn start_before(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        lane: &LaneRecord,
        argv: &[String],
        key: &str,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let rec = &t.services[i];
        let mut dir = self
            .data
            .ticket_dir(&t.id)
            .join(&rec.stage)
            .join("services")
            .join(&rec.lane);
        if let Some(process) = &rec.process {
            dir.push(process);
        }
        dir.push(rec.n.to_string());
        std::fs::create_dir_all(&dir)?;
        let log = dir.join("before.log");
        let head = self.git.head(&lane.worktree)?;
        let env = env_for(t, Some(&lane.name), Some(&lane.branch));
        // Confined as the lane's checks are, when the pipeline says so.
        let started = match confine_for(t, p, Some(&lane.name), &[&dir], None) {
            Some(confine) => {
                self.git
                    .start_check_confined(key, &lane.worktree, argv, &env, &log, &confine, &[])
            }
            None => self
                .git
                .start_check(key, &lane.worktree, argv, &env, &log, &[]),
        };
        if let Err(e) = started {
            return self.fail_service(t, i, &format!("before could not start: {e:#}"), now_ms);
        }
        log::info!("ticket {} before for {} started ({key})", t.id, lane.name);
        t.services[i].before = Some(GateRun {
            head,
            argv: argv.to_vec(),
            log,
            started_ms: now_ms,
            exit: None,
            group: self.git.check_group(key),
            lost_since_ms: None,
            env: false,
        });
        self.save_ticket(t, now_ms)
    }

    /// The lowest port of the policy's range on no live service record
    /// of any ticket that binds on this machine.
    fn free_port(&self, t: &Ticket, p: &Pipeline) -> Result<Option<u16>> {
        let Some([lo, hi]) = p.policy.ports else {
            return Ok(None);
        };
        let mut taken: BTreeSet<u16> = live_ports(t).collect();
        for other in self.tickets()? {
            if other.id != t.id {
                taken.extend(live_ports(&other));
            }
        }
        Ok((lo..=hi).find(|port| !taken.contains(port) && self.git.port_free(*port)))
    }

    /// The service's launch: a Switchboard service session in the
    /// ticket's project, in the lane's tree, run through the login shell
    /// so the user's PATH finds `npm`. The environment and command are
    /// argv elements, never shell source; only the pipeline file's
    /// `serve.env` values, the port, and the URL or port of a lane they
    /// name reach them.
    fn send_service(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        process: &Process,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let project = match self.ensure_project(t, ps, p, now_ms) {
            Ok(id) => id,
            Err(e) if e.is::<SocketDown>() => return Err(e),
            Err(e) => {
                return self.fail_service(t, i, &format!("no project for it: {e:#}"), now_ms);
            }
        };
        let rec = t.services[i].clone();
        let port = rec.port.unwrap_or_default().to_string();
        // A named lane with a record on its way is waited for; the next
        // pass sends again. Only a lane with no live record fails below.
        let named_lanes = process.named_lanes();
        if named_lanes.iter().any(|lane| {
            latest_live(t, p, &rec.stage, lane).is_some() && !lane_ready(t, p, &rec.stage, lane)
        }) {
            return Ok(());
        }
        let mut pointed_at = BTreeMap::new();
        for lane in named_lanes {
            if let Some(named) = latest_live(t, p, &rec.stage, &lane) {
                pointed_at.insert(lane, named.n);
            }
        }
        let mut env = Vec::new();
        for (k, v) in &process.env {
            let value = render_env_value(v, &port, |r| {
                let named = latest_live(t, p, &rec.stage, r.lane()?)?;
                match r {
                    ServeRef::PortOf(_) => named.port.map(|p| p.to_string()),
                    _ => named.url.clone(),
                }
            });
            match value {
                Ok(value) => env.push(format!("{k}={value}")),
                Err(field) => {
                    let reason = format!("{field} does not answer yet");
                    return self.fail_service(t, i, &reason, now_ms);
                }
            }
        }
        // Saved before the send, so a resend after a crash is checked
        // against what it will use.
        if t.services[i].pointed_at != pointed_at {
            t.services[i].pointed_at = pointed_at;
            self.save_ticket(t, now_ms)?;
        }
        let Some(cwd) = t
            .lanes
            .iter()
            .find(|l| l.name == rec.lane)
            .map(|l| l.worktree.clone())
        else {
            return self.fail_service(t, i, "its lane has no tree", now_ms);
        };
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/zsh".to_owned());
        let mut argv = vec![
            shell,
            "-lc".to_owned(),
            "exec \"$@\"".to_owned(),
            "dispatch-service".to_owned(),
            "env".to_owned(),
        ];
        argv.extend(env);
        argv.extend(process.argv.iter().map(|a| a.replace("{port}", &port)));
        let label = rec.label();
        let (name, on) = match (rec.port, &rec.url) {
            (Some(p), Some(url)) => (format!("{label} :{p}"), format!(" on {url}")),
            (Some(p), None) => (format!("{label} :{p}"), format!(" on port {p}")),
            (None, _) => (label.clone(), String::new()),
        };
        let body = Body::SessionNew {
            project,
            name,
            session_kind: wire::SessionKind::Service,
            cwd,
            launch: wire::Launch::Argv(argv),
            prompt: None,
            notes: format!(
                "Dispatch ticket {} ({}) serves {label} for {}{on}",
                t.id,
                t.source.label(),
                rec.stage
            ),
            env: std::collections::BTreeMap::new(),
            env_sets: Vec::new(),
            replaces: None,
        };
        let reply = self.send(t, ps, None, &rec.intent(), body, now_ms)?;
        if let Reply::Failed { reason } = reply {
            return self.fail_service(t, i, &format!("could not start: {reason}"), now_ms);
        }
        Ok(())
    }

    /// A launched service: its launch settled, then the probe until it
    /// answers, or with no probe its session running. A session that is
    /// gone, or one that does not answer within the probe's
    /// `within_secs`, fails the record; the failure stops it before
    /// anything is asked. The pane's process group is read once here,
    /// for the stop to check.
    fn service_starting(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        process: &Process,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let rec = t.services[i].clone();
        let Some(session) = rec.session.clone() else {
            let intent = rec.intent();
            let entry = t.ledger.iter().rev().find(|o| o.intent == intent).cloned();
            return match entry {
                // Saved `Starting`, then stopped before the request was
                // written: nothing went out, so it is sent now.
                None => self.send_service(t, ps, p, process, i, now_ms),
                // Recovery resolves it through `find`.
                Some(o) if o.unresolved() => Ok(()),
                Some(o) => {
                    t.services[i].op = Some(o.op.clone());
                    let made = o.reply.as_ref().map(|r| r.made().to_vec());
                    if let Some(made) = made.filter(|m| !m.is_empty()) {
                        apply_service_reply(t, &intent, &made);
                        if t.services[i].session.is_some() {
                            return self.save_ticket(t, now_ms);
                        }
                    }
                    let reason = match &o.reply {
                        Some(Reply::Failed { reason }) => format!("could not start: {reason}"),
                        _ => "its launch was lost".to_owned(),
                    };
                    self.fail_service(t, i, &reason, now_ms)
                }
            };
        };
        let view = self.session_view(&session)?;
        let running = matches!(&view, Ok(v) if v.liveness == wire::Liveness::Running);
        let gone = session_gone(&view);
        if running
            && rec.group.is_none()
            && let Some(pid) = view.as_ref().ok().and_then(|v| v.pid)
            && let Some(group) = self.git.group_of(pid)
        {
            t.services[i].group = Some(group);
            self.save_ticket(t, now_ms)?;
        }
        let up = match &process.ready {
            Some(ready) => self
                .git
                .answers_http(rec.port.unwrap_or_default(), &ready.http),
            None => running,
        };
        if up {
            t.services[i].state = ServiceState::Ready;
            t.services[i].ready_ms = Some(now_ms);
            log::info!(
                "ticket {} {} ready{}",
                t.id,
                rec.label(),
                rec.url
                    .as_deref()
                    .map(|u| format!(" at {u}"))
                    .unwrap_or_default()
            );
            return self.save_ticket(t, now_ms);
        }
        if gone {
            return self.fail_service(t, i, "the service exited", now_ms);
        }
        let Some(within) = process.ready.as_ref().map(|r| r.within_secs) else {
            return Ok(());
        };
        if now_ms > rec.started_ms.saturating_add(within.saturating_mul(1000)) {
            let url = rec.url.unwrap_or_default();
            self.fail_service(
                t,
                i,
                &format!("{url} did not answer within {within}s"),
                now_ms,
            )
        } else {
            Ok(())
        }
    }

    /// A served process of a `serve` list, read again each pass: one
    /// whose session is no longer running fails the lane, and the owner
    /// is asked, since what the agent is trying is now half gone. The
    /// single table is not read again once ready; it reads as up until
    /// its stop, as it did before lists, while a list's other processes
    /// depend on each one staying up.
    fn service_running(&mut self, t: &mut Ticket, i: usize, now_ms: u64) -> Result<()> {
        let Some(session) = t.services[i].session.clone() else {
            return Ok(());
        };
        if !session_gone(&self.session_view(&session)?) {
            return Ok(());
        }
        let name = t.services[i]
            .process
            .clone()
            .unwrap_or_else(|| t.services[i].lane.clone());
        self.fail_service(t, i, &format!("{name} exited"), now_ms)
    }

    fn fail_service(&mut self, t: &mut Ticket, i: usize, reason: &str, now_ms: u64) -> Result<()> {
        let rec = &mut t.services[i];
        log::warn!(
            "ticket {} {} service {} failed: {reason}",
            t.id,
            rec.stage,
            rec.label()
        );
        rec.state = ServiceState::Failed {
            reason: reason.to_owned(),
        };
        self.save_ticket(t, now_ms)
    }

    /// One service stopped: its `before` waited for (not killed: it may
    /// have descendants a kill would not reach), its session killed and
    /// read back gone, its port binding again, then the session removed
    /// and the record written `Stopped`. True once it is. Past
    /// `STOP_LIMIT_MS` with something still alive, the `stuck` question
    /// is asked; only its `released` answer ends the wait without the
    /// runner's confirmation.
    pub(crate) fn stop_service(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        i: usize,
        now_ms: u64,
    ) -> Result<bool> {
        self.stop_service_into(t, ps, i, ServiceState::Stopped, now_ms)
    }

    /// `stop_service`, ending in `end`: a failed service keeps its
    /// failure, to be asked about, once it is confirmed gone.
    fn stop_service_into(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        i: usize,
        end: ServiceState,
        now_ms: u64,
    ) -> Result<bool> {
        if t.services[i].state == ServiceState::Stopped {
            return Ok(true);
        }
        // Read here, not by `act_on_answers`, which never runs for a
        // parking or closing ticket.
        let (stage, key) = (t.services[i].stage.clone(), stuck_key(&t.services[i]));
        if let Some((d, answer)) = stuck_answer(t, &stage, &key) {
            if let DecisionState::Answered { acted, .. } = &mut t.decisions[d].state {
                *acted = true;
            }
            if answer == "released" {
                self.released_by_hand(t, ps, i, end, now_ms)?;
                return Ok(true);
            }
            // `wait`: the limit runs again from now.
            t.services[i].stopping_ms = Some(now_ms);
            self.save_ticket(t, now_ms)?;
            return Ok(false);
        }
        let Some(alive) = self.still_alive(t, ps, i, now_ms)? else {
            self.remove_service_session(t, ps, i, now_ms)?;
            // A `stuck` asked while it was slow has nothing left to be
            // about, and no later stop would read its answer.
            let withdrawn = withdraw_stuck(t, &stage, &key);
            let rec = &mut t.services[i];
            let ended = rec.state != end;
            if ended {
                log::info!(
                    "ticket {} {} service {} stopped",
                    t.id,
                    rec.stage,
                    rec.label()
                );
                rec.state = end;
            }
            if ended || withdrawn {
                self.save_ticket(t, now_ms)?;
            }
            return Ok(true);
        };
        match t.services[i].stopping_ms {
            None => {
                t.services[i].stopping_ms = Some(now_ms);
                self.save_ticket(t, now_ms)?;
            }
            Some(since)
                if now_ms.saturating_sub(since) >= STOP_LIMIT_MS
                    && !stuck_pending(t, &stage, &key) =>
            {
                self.ask_stuck(t, i, &alive, now_ms)?;
            }
            Some(_) => {}
        }
        Ok(false)
    }

    /// What of the service is still alive, or `None` once nothing is:
    /// its launch in flight, its `before`, its session, or its port.
    fn still_alive(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        i: usize,
        now_ms: u64,
    ) -> Result<Option<String>> {
        let rec = t.services[i].clone();
        // A launch still in flight would bring the session up after the
        // stop read it as gone.
        if rec.session.is_none()
            && t.ledger
                .iter()
                .rev()
                .find(|o| o.intent == rec.intent())
                .is_some_and(crate::ticket::Operation::unresolved)
        {
            return Ok(Some("its launch, still in flight".to_owned()));
        }
        if let Some(before) = &rec.before
            && before.exit.is_none()
        {
            let key = before_key(t, &rec);
            match self.git.poll_check(&key) {
                None => {
                    return Ok(Some(format!(
                        "its before ({key}, log at {})",
                        before.log.display()
                    )));
                }
                Some(Ok(code)) => {
                    if let Some(b) = &mut t.services[i].before {
                        b.exit = Some(code);
                    }
                    self.save_ticket(t, now_ms)?;
                }
                // Gone with a runner that restarted.
                Some(Err(_)) => {}
            }
        }
        if let Some(session) = rec.session.as_ref().filter(|s| t.processes.contains(s))
            && !self.retire_processes(t, ps, std::slice::from_ref(session), now_ms)?
        {
            return Ok(Some(format!("its session {session}")));
        }
        // A dev server's forked child can outlive its pane and keep the
        // port, so the port binding again is part of stopped. Removing
        // the session confirmed the stop; whatever binds the port after
        // that is not this service.
        if let Some(port) = rec.port
            && rec
                .session
                .as_ref()
                .is_some_and(|s| t.processes.contains(s))
            && !self.git.port_free(port)
        {
            return Ok(Some(format!(
                "port {port} is still taken (look with `lsof -i :{port}`)"
            )));
        }
        // A process that ignored the pane's hang-up, or left with its
        // group, holds no port to see it by; its group is how.
        if let Some(group) = &rec.group
            && rec
                .session
                .as_ref()
                .is_some_and(|s| t.processes.contains(s))
            && self.git.group_running(&rec.intent(), group)
        {
            let pgid = group.pgid;
            return Ok(Some(format!(
                "process group {pgid} still has members (look with `pgrep -g {pgid}`)"
            )));
        }
        Ok(None)
    }

    /// The service's session removed from Switchboard once it is gone:
    /// a record Dispatch made is Dispatch's to remove. Sent once; the
    /// session leaves the ticket's process list with it.
    fn remove_service_session(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let Some(session) = t.services[i]
            .session
            .clone()
            .filter(|s| t.processes.contains(s))
        else {
            return Ok(());
        };
        self.send(
            t,
            ps,
            None,
            "remove",
            Body::SessionRemove {
                session: session.clone(),
            },
            now_ms,
        )?;
        t.processes.retain(|s| s != &session);
        self.save_ticket(t, now_ms)
    }

    /// A `released` answer: what the runner could not confirm, the user
    /// did. A `before` still held is killed (best effort), the session
    /// removed, and the record ends with what was named.
    fn released_by_hand(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        i: usize,
        end: ServiceState,
        now_ms: u64,
    ) -> Result<()> {
        let rec = t.services[i].clone();
        let what = rec
            .stuck_on
            .clone()
            .unwrap_or_else(|| "the service".to_owned());
        if rec.before.as_ref().is_some_and(|b| b.exit.is_none()) {
            self.git.kill_check(&before_key(t, &rec));
        }
        self.remove_service_session(t, ps, i, now_ms)?;
        log::warn!(
            "ticket {} {} service {} released by hand: {what}",
            t.id,
            rec.stage,
            rec.label()
        );
        withdraw_stuck(t, &rec.stage, &stuck_key(&rec));
        let rec = &mut t.services[i];
        rec.released = Some(what);
        rec.state = end;
        self.save_ticket(t, now_ms)
    }

    /// The service's `stuck` question, with what is still alive kept on
    /// the record for a `released` to name.
    fn ask_stuck(&mut self, t: &mut Ticket, i: usize, alive: &str, now_ms: u64) -> Result<()> {
        let rec = t.services[i].clone();
        let question = format!(
            "{} ({}): the service has not stopped after {}s: {alive}. Stop it by hand and answer released, or answer wait to give it longer. The hold on what the stage needs stays until then.",
            rec.stage,
            rec.label(),
            STOP_LIMIT_MS / 1000
        );
        t.services[i].stuck_on = Some(alive.to_owned());
        push_stuck(t, &rec.stage, stuck_key(&rec), question, now_ms);
        self.save_ticket(t, now_ms)
    }
}

/// A hold taken past the first stage of its run (a resume, after
/// parking let it go) cannot trust what the run's earlier stages did:
/// another ticket may have deployed since, and parking stopped the
/// services the tester ran against. Every stage earlier in the run has
/// its completed attempts cancelled, so a command or agent stage is a
/// `rerun` question rather than a result (an agent stage asks once its
/// services are up again) and a human gate asks afresh, and the
/// earliest is the stage the ticket goes back to. An earlier run of the
/// same resource is not touched: it was let go at its end by design.
/// Validation keeps review and workflow stages out of this span. `None`
/// when nothing in the run ran.
fn undo_range(t: &mut Ticket, p: &Pipeline, resource: &str, now_ms: u64) -> Option<usize> {
    let (first, _) = p.hold_run_at(resource, t.stage)?;
    let mut back = None;
    for i in first..t.stage {
        let name = &p.stages[i].name;
        for a in &mut t.attempts {
            if &a.stage == name && a.state == AttemptState::Complete {
                a.state = AttemptState::Cancelled {
                    reason: format!(
                        "{resource} was let go after this stage, so what it saw may have been replaced by another ticket"
                    ),
                };
                a.ended_ms = Some(now_ms);
                back.get_or_insert(i);
            }
        }
    }
    back
}

/// What `advance_lane` needs of the lane beyond its processes: the
/// pipeline, the stage, and the ticket's record of the lane.
struct LaneCx<'a> {
    p: &'a Pipeline,
    stage: &'a Stage,
    record: &'a LaneRecord,
}

/// Whether a session read says the session is gone: not running, or no
/// such session. Any other failed read is neither gone nor running; the
/// next pass reads again.
fn session_gone(view: &std::result::Result<wire::SessionView, String>) -> bool {
    match view {
        Ok(v) => v.liveness != wire::Liveness::Running,
        Err(reason) => reason == NO_SUCH_SESSION,
    }
}

fn is_failed(rec: &ServiceRecord) -> bool {
    matches!(rec.state, ServiceState::Failed { .. })
}

/// The URL the tester is given for `lane` in `stage`: the first
/// process's, once every process of the lane has a `Ready` record for
/// the stage; `None` until then.
#[must_use]
pub(crate) fn lane_url(t: &Ticket, p: &Pipeline, stage: &str, lane: &str) -> Option<String> {
    let processes = p.lane(lane)?.serve.as_ref()?.processes();
    let ready: Vec<&ServiceRecord> = processes
        .iter()
        .map(|process| {
            t.services.iter().rev().find(|x| {
                x.is_for(stage, lane, process.name.as_deref()) && x.state == ServiceState::Ready
            })
        })
        .collect::<Option<_>>()?;
    ready.first()?.url.clone()
}

/// Whether the record's range (its stage to its `until`) holds stage
/// index `stage`; past the last stage none does.
fn in_range(p: &Pipeline, s: &ServiceRecord, stage: usize) -> bool {
    let index = |name: &str| p.stages.iter().position(|x| x.name == name);
    match (index(&s.stage), index(&s.until)) {
        (Some(first), Some(last)) => (first..=last).contains(&stage),
        _ => false,
    }
}

/// The key a service's `before` is polled under: the record's number in
/// it, so a retry's record never reads an earlier one's exit.
pub(crate) fn before_key(t: &Ticket, rec: &ServiceRecord) -> String {
    format!("{}/{}/before:{}:{}", t.id, rec.stage, rec.label(), rec.n)
}

/// What a `stuck` question carries as its "attempt": not an attempt of
/// any stage (`held_in` finds none, so it holds nothing), but the
/// record it is about.
fn stuck_key(rec: &ServiceRecord) -> (String, u32) {
    (format!("service:{}", rec.label()), rec.n)
}

/// Whether `d` is the `stuck` question about `key` in `stage`: a
/// service's or a lost command's, each with its own pseudo-key.
fn is_stuck(d: &Decision, stage: &str, key: &(String, u32)) -> bool {
    d.name == STUCK && d.stage == stage && d.attempt.as_ref() == Some(key)
}

/// The unacted answer to the `stuck` question about `key`, with its
/// index.
pub(crate) fn stuck_answer(
    t: &Ticket,
    stage: &str,
    key: &(String, u32),
) -> Option<(usize, String)> {
    t.decisions.iter().enumerate().find_map(|(d, x)| {
        x.unacted_answer()
            .filter(|_| is_stuck(x, stage, key))
            .map(|a| (d, a.to_owned()))
    })
}

pub(crate) fn stuck_pending(t: &Ticket, stage: &str, key: &(String, u32)) -> bool {
    t.decisions
        .iter()
        .any(|d| d.pending() && is_stuck(d, stage, key))
}

/// Each pending `stuck` question about `key` withdrawn; true if there
/// was one.
pub(crate) fn withdraw_stuck(t: &mut Ticket, stage: &str, key: &(String, u32)) -> bool {
    let mut withdrawn = false;
    for d in &mut t.decisions {
        if d.pending() && is_stuck(d, stage, key) {
            d.state = DecisionState::Cancelled;
            withdrawn = true;
        }
    }
    withdrawn
}

/// A `stuck` question about `key`, pushed straight onto the record and
/// logged: asking through `ensure_decision` would mark the current
/// session waiting, which may be the one being stopped, and which
/// parking and the close unmark.
pub(crate) fn push_stuck(
    t: &mut Ticket,
    stage: &str,
    key: (String, u32),
    question: String,
    now_ms: u64,
) {
    let id = format!("d{}", t.decisions.len() + 1);
    log::warn!("ticket {} decision {id} ({STUCK}): {question}", t.id);
    t.decisions.push(Decision {
        id,
        stage: stage.to_owned(),
        name: STUCK.to_owned(),
        kind: DecisionKind::Permission,
        question,
        options: vec!["wait".to_owned(), "released".to_owned()],
        recommendation: None,
        attempt: Some(key),
        state: DecisionState::Pending,
        made_ms: now_ms,
        refusals: Vec::new(),
    });
}

/// The ports of a ticket's services not yet stopped.
fn live_ports(t: &Ticket) -> impl Iterator<Item = u16> + '_ {
    t.services
        .iter()
        .filter(|s| s.state != ServiceState::Stopped)
        .filter_map(|s| s.port)
}

/// The stage's record of `lane` that is not stopped, the latest, of
/// the lane's first process: the one whose URL and port another lane's
/// `serve.env` names.
fn latest_live<'a>(
    t: &'a Ticket,
    p: &Pipeline,
    stage: &str,
    lane: &str,
) -> Option<&'a ServiceRecord> {
    let first = p
        .lane(lane)
        .and_then(|l| l.serve.as_ref())
        .and_then(|s| s.processes().first().and_then(|x| x.name.clone()));
    t.services
        .iter()
        .rev()
        .find(|s| s.is_for(stage, lane, first.as_deref()) && s.state != ServiceState::Stopped)
}

/// Whether every process of `lane` has a live record that answers in
/// `stage`.
fn lane_ready(t: &Ticket, p: &Pipeline, stage: &str, lane: &str) -> bool {
    let Some(spec) = p.lane(lane).and_then(|l| l.serve.as_ref()) else {
        return false;
    };
    spec.processes().iter().all(|process| {
        t.services
            .iter()
            .rev()
            .find(|s| {
                s.is_for(stage, lane, process.name.as_deref()) && s.state != ServiceState::Stopped
            })
            .is_some_and(|s| s.state == ServiceState::Ready)
    })
}

/// Whether record `i` was launched against a record of a lane it names
/// that is no longer that lane's live one. A lane that only failed keeps
/// its number, so its dependent is restarted once a retry makes the next.
fn stale_pointer(t: &Ticket, p: &Pipeline, i: usize) -> bool {
    let rec = &t.services[i];
    rec.pointed_at
        .iter()
        .any(|(lane, n)| latest_live(t, p, &rec.stage, lane).map(|s| s.n) != Some(*n))
}

/// The first field of a process's `env` naming a lane the ticket did
/// not choose, so nothing will ever serve it, with the lane's name.
fn unserved_ref(t: &Ticket, spec: &ServeSpec) -> Option<(ServeRef, String)> {
    spec.refs().into_iter().find_map(|r| {
        let lane = r.lane()?.to_owned();
        (!t.lanes.iter().any(|l| l.name == lane && l.chosen)).then_some((r, lane))
    })
}

/// Whether every lane the processes' `env` names answers in `stage`.
fn named_ready(t: &Ticket, p: &Pipeline, stage: &str, spec: &ServeSpec) -> bool {
    spec.named_lanes()
        .iter()
        .all(|lane| lane_ready(t, p, stage, lane))
}

/// `value` with `{port}` filled from `port` and each other field from
/// `lookup`. The `Err` is the first field `lookup` has nothing for.
fn render_env_value(
    value: &str,
    port: &str,
    mut lookup: impl FnMut(&ServeRef) -> Option<String>,
) -> std::result::Result<String, String> {
    let mut out = value.to_owned();
    for r in ServeRef::parse(value) {
        let filled = match &r {
            ServeRef::Port => port.to_owned(),
            ServeRef::Url(_) | ServeRef::PortOf(_) => lookup(&r).ok_or_else(|| r.token())?,
            ServeRef::Unknown(field) => return Err(field.clone()),
        };
        out = out.replace(&r.token(), &filled);
    }
    Ok(out)
}

/// The other tickets of `t`'s project that hold `resource`, as
/// `<id> (<source>) from <stage>`, the stage each holds it from.
fn holders_of(all: &[Ticket], t: &Ticket, resource: &str) -> Vec<String> {
    all.iter()
        .filter(|o| o.id != t.id && o.project == t.project)
        .filter(|o| !matches!(o.state, TicketState::Closed { .. }))
        .filter_map(|o| {
            let h = o.holds.iter().find(|h| h.resource == resource)?;
            Some(format!("{} ({}) from {}", o.id, o.source.label(), h.stage))
        })
        .collect()
}

/// What an active ticket waits for: a resource its current stage needs
/// that as many other tickets hold as its `count` allows, and who holds
/// it from which stage, as `my-dev, held by baea8dbe (#56) from
/// implement`.
#[must_use]
pub fn waiting_for(t: &Ticket, p: &Pipeline, all: &[Ticket]) -> Option<String> {
    if !t.active() {
        return None;
    }
    let stage = p.stages.get(t.stage)?;
    stage
        .needs
        .iter()
        .filter(|n| !t.holds.iter().any(|h| &h.resource == *n))
        .find_map(|need| {
            let resource = p.resource(need)?;
            let holders = holders_of(all, t, need);
            (holders.len() >= resource.count as usize)
                .then(|| format!("{need}, held by {}", holders.join(", ")))
        })
}

/// A `session.new` reply under a service intent, now or from recovery:
/// the operation and the session it made go on the record, and the
/// session on the ticket's process list.
pub(crate) fn apply_service_reply(t: &mut Ticket, intent: &str, made: &[Made]) {
    let Some(op) = t
        .ledger
        .iter()
        .rev()
        .find(|o| o.intent == intent)
        .map(|o| o.op.clone())
    else {
        return;
    };
    let session = made
        .iter()
        .find(|m| m.kind == wire::RecordKind::Session)
        .map(|m| m.id.clone());
    let Some(rec) = t.services.iter_mut().find(|s| s.intent() == intent) else {
        return;
    };
    rec.op = Some(op);
    if let Some(id) = session {
        rec.session.get_or_insert_with(|| id.clone());
        if !t.processes.contains(&id) {
            t.processes.push(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_env_value_takes_its_own_port_and_a_named_lanes_fields() {
        let lookup = |r: &ServeRef| match r {
            ServeRef::Url(l) if l == "backend" => Some("http://127.0.0.1:3101".to_owned()),
            ServeRef::PortOf(l) if l == "backend" => Some("3101".to_owned()),
            _ => None,
        };
        assert_eq!(
            render_env_value("{services.backend}/api", "3100", lookup),
            Ok("http://127.0.0.1:3101/api".to_owned())
        );
        assert_eq!(
            render_env_value("{port}:{ports.backend}", "3100", lookup),
            Ok("3100:3101".to_owned())
        );
        assert_eq!(
            render_env_value("plain", "3100", lookup),
            Ok("plain".to_owned())
        );
        assert_eq!(
            render_env_value("{services.api}", "3100", lookup),
            Err("{services.api}".to_owned())
        );
    }
}
