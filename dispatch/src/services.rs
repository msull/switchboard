//! A stage's resources and services. A ticket takes a hold on each
//! `[[resources]]` entry its stage `needs` on entering the range of
//! stages that need it, and drops it the pass after it leaves, once the
//! services started inside the range are confirmed stopped. A stage's
//! `services` are the lanes it serves: the lane's `before` as a child of
//! the runner, a free port, a Switchboard service session launched with
//! the port in its environment, and the readiness probe. Both live on
//! the ticket record; the scheduler steps them.

use std::collections::BTreeSet;

use anyhow::Result;
use switchboard_control::{self as wire, Body, Made, Reply};

use crate::pipeline::{Pipeline, Serve, Stage, StageKind};
use crate::scheduler::{Ask, NO_SUCH_SESSION, Runner, SocketDown, confine_for, env_for};
use crate::ticket::{
    AttemptState, Decision, DecisionKind, DecisionState, GateRun, Hold, LaneRecord, ProjectState,
    STUCK, ServiceRecord, ServiceState, Ticket, TicketState,
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
        let (kept, dropped): (Vec<Hold>, Vec<Hold>) =
            t.holds.drain(..).partition(|h| needs.contains(&h.resource));
        t.holds = kept;
        if dropped.is_empty() {
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

    // --- services

    /// The stage's services, in order, brought one step further each
    /// pass. A lane the ticket did not choose is not served. True once
    /// every served lane answers; until then no agent of the stage
    /// launches. A lane that could not be served is stopped, then asked
    /// about once (`retry` or `park`).
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
        for lane in &stage.services {
            let Some(record) = t
                .lanes
                .iter()
                .find(|l| &l.name == lane && l.chosen)
                .cloned()
            else {
                continue;
            };
            let Some(serve) = p.lane(lane).and_then(|l| l.serve.clone()) else {
                continue;
            };
            let i = self.service_record(t, p, &stage.name, lane, now_ms)?;
            match t.services[i].state.clone() {
                ServiceState::Before => {
                    self.service_before(t, ps, p, stage, &record, &serve, i, now_ms)?;
                }
                ServiceState::Starting => self.service_starting(t, ps, p, &serve, i, now_ms)?,
                ServiceState::Ready | ServiceState::Failed { .. } | ServiceState::Stopped => {}
            }
            if !t.active() {
                return Ok(false);
            }
            match t.services[i].state.clone() {
                ServiceState::Ready => {}
                // Asked about only once it is confirmed gone, so a
                // `retry` never starts a second one beside it.
                ServiceState::Failed { reason } => {
                    ready = false;
                    let end = ServiceState::Failed {
                        reason: reason.clone(),
                    };
                    if self.stop_service_into(t, ps, i, end, now_ms)? {
                        failed.push(format!("{lane}: {reason}"));
                    }
                }
                _ => ready = false,
            }
            if !t.active() {
                return Ok(false);
            }
        }
        if !failed.is_empty() {
            self.ensure_decision(
                t,
                ps,
                Ask {
                    stage: &stage.name,
                    name: SERVICE,
                    kind: DecisionKind::Permission,
                    question: format!(
                        "{}: could not serve {}. Retry starts each again (its before, a new port), park stops the ticket.",
                        stage.name,
                        failed.join("; ")
                    ),
                    options: &["retry", "park"],
                    recommendation: None,
                    attempt: None,
                },
                now_ms,
            )?;
        }
        Ok(ready)
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

    /// The index of the stage's live record for `lane`, made in `Before`
    /// when there is none.
    fn service_record(
        &mut self,
        t: &mut Ticket,
        p: &Pipeline,
        stage: &str,
        lane: &str,
        now_ms: u64,
    ) -> Result<usize> {
        if let Some(i) = t
            .services
            .iter()
            .rposition(|s| s.stage == stage && s.lane == lane && s.state != ServiceState::Stopped)
        {
            return Ok(i);
        }
        let n = t
            .services
            .iter()
            .filter(|s| s.stage == stage && s.lane == lane)
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
        });
        self.save_ticket(t, now_ms)?;
        Ok(t.services.len() - 1)
    }

    /// The lane's `before`, started, watched, and once it exits 0 (or
    /// with none) a free port and the launch.
    #[allow(clippy::too_many_arguments)]
    fn service_before(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        stage: &Stage,
        lane: &LaneRecord,
        serve: &Serve,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
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
        let Some(port) = self.free_port(t, p)? else {
            let range = p
                .policy
                .ports
                .map(|[lo, hi]| format!("{lo}-{hi}"))
                .unwrap_or_default();
            return self.fail_service(t, i, &format!("no free port in {range}"), now_ms);
        };
        let rec = &mut t.services[i];
        rec.port = Some(port);
        rec.url = Some(serve.url.replace("{port}", &port.to_string()));
        rec.state = ServiceState::Starting;
        rec.started_ms = now_ms;
        log::info!(
            "ticket {} {} serves {} on port {port}",
            t.id,
            stage.name,
            lane.name
        );
        self.save_ticket(t, now_ms)?;
        self.send_service(t, ps, p, serve, i, now_ms)
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
        let dir = self
            .data
            .ticket_dir(&t.id)
            .join(&rec.stage)
            .join("services")
            .join(&rec.lane)
            .join(rec.n.to_string());
        std::fs::create_dir_all(&dir)?;
        let log = dir.join("before.log");
        let head = self.git.head(&lane.worktree)?;
        let env = env_for(t, Some(&lane.name), Some(&lane.branch));
        // Confined as the lane's checks are, when the pipeline says so.
        let started = match confine_for(t, p, Some(&lane.name), &[&dir], None) {
            Some(confine) => {
                self.git
                    .start_check_confined(key, &lane.worktree, argv, &env, &log, &confine)
            }
            None => self.git.start_check(key, &lane.worktree, argv, &env, &log),
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
    /// `serve.env` values and the port reach them.
    fn send_service(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        serve: &Serve,
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
        argv.extend(
            serve
                .env
                .iter()
                .map(|(k, v)| format!("{k}={}", v.replace("{port}", &port))),
        );
        argv.extend(serve.argv.iter().cloned());
        let url = rec.url.clone().unwrap_or_default();
        let body = Body::SessionNew {
            project,
            name: format!("{} :{port}", rec.lane),
            session_kind: wire::SessionKind::Service,
            cwd,
            launch: wire::Launch::Argv(argv),
            prompt: None,
            notes: format!(
                "Dispatch ticket {} ({}) serves {} for {} on {url}",
                t.id,
                t.source.label(),
                rec.lane,
                rec.stage
            ),
        };
        let reply = self.send(t, ps, None, &rec.intent(), body, now_ms)?;
        if let Reply::Failed { reason } = reply {
            return self.fail_service(t, i, &format!("could not start: {reason}"), now_ms);
        }
        Ok(())
    }

    /// A launched service: its launch settled, then the probe until it
    /// answers. A session that is gone, or one that does not answer
    /// within the lane's `ready.within_secs`, fails the record; the
    /// failure stops it before anything is asked.
    fn service_starting(
        &mut self,
        t: &mut Ticket,
        ps: &mut ProjectState,
        p: &Pipeline,
        serve: &Serve,
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
                None => self.send_service(t, ps, p, serve, i, now_ms),
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
        let port = rec.port.unwrap_or_default();
        if self.git.answers_http(port, &serve.ready.http) {
            t.services[i].state = ServiceState::Ready;
            t.services[i].ready_ms = Some(now_ms);
            log::info!(
                "ticket {} {} ready at {}",
                t.id,
                rec.lane,
                rec.url.as_deref().unwrap_or_default()
            );
            return self.save_ticket(t, now_ms);
        }
        let gone = match self.session_view(&session)? {
            Ok(view) => view.liveness != wire::Liveness::Running,
            Err(reason) => reason == NO_SUCH_SESSION,
        };
        let within = serve.ready.within_secs;
        if gone {
            self.fail_service(t, i, "the service exited", now_ms)
        } else if now_ms > rec.started_ms.saturating_add(within.saturating_mul(1000)) {
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

    fn fail_service(&mut self, t: &mut Ticket, i: usize, reason: &str, now_ms: u64) -> Result<()> {
        let rec = &mut t.services[i];
        log::warn!(
            "ticket {} {} service {} failed: {reason}",
            t.id,
            rec.stage,
            rec.lane
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
        if let Some((d, answer)) = stuck_answer(t, i) {
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
            let withdrawn = withdraw_stuck(t, i);
            let rec = &mut t.services[i];
            let ended = rec.state != end;
            if ended {
                log::info!("ticket {} {} service {} stopped", t.id, rec.stage, rec.lane);
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
                if now_ms.saturating_sub(since) >= STOP_LIMIT_MS && !stuck_pending(t, i) =>
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
            rec.lane
        );
        withdraw_stuck(t, i);
        let rec = &mut t.services[i];
        rec.released = Some(what);
        rec.state = end;
        self.save_ticket(t, now_ms)
    }

    /// The `stuck` question, written straight to the record: asking
    /// through `ensure_decision` would mark the current session waiting,
    /// which may be the one being stopped, and which parking and the
    /// close unmark.
    fn ask_stuck(&mut self, t: &mut Ticket, i: usize, alive: &str, now_ms: u64) -> Result<()> {
        let rec = t.services[i].clone();
        let id = format!("d{}", t.decisions.len() + 1);
        let question = format!(
            "{} ({}): the service has not stopped after {}s: {alive}. Stop it by hand and answer released, or answer wait to give it longer. The hold on what the stage needs stays until then.",
            rec.stage,
            rec.lane,
            STOP_LIMIT_MS / 1000
        );
        log::warn!("ticket {} decision {id} ({STUCK}): {question}", t.id);
        t.services[i].stuck_on = Some(alive.to_owned());
        t.decisions.push(Decision {
            id,
            stage: rec.stage.clone(),
            name: STUCK.to_owned(),
            kind: DecisionKind::Permission,
            question,
            options: vec!["wait".to_owned(), "released".to_owned()],
            recommendation: None,
            attempt: Some(stuck_key(&rec)),
            state: DecisionState::Pending,
            made_ms: now_ms,
            refusals: Vec::new(),
        });
        self.save_ticket(t, now_ms)
    }
}

/// A hold taken past the first stage of its range (a resume, after
/// parking let it go): what the range's stages did with the resource
/// may have been replaced by another ticket's since, a deploy, and the
/// tester that ran against it with services that parking stopped. Each
/// command or agent stage earlier in the range has its completed
/// attempts cancelled, so it is a `rerun` question rather than a
/// result (and an agent stage brings its services up again before
/// asking), and the earliest is the stage the ticket goes back to.
/// `None` when nothing in the range ran.
fn undo_range(t: &mut Ticket, p: &Pipeline, resource: &str, now_ms: u64) -> Option<usize> {
    let (first, _) = p.hold_range(resource)?;
    let mut back = None;
    for i in first..t.stage {
        let s = &p.stages[i];
        if !s.is_command_stage() && s.kind() != StageKind::Agent {
            continue;
        }
        for a in &mut t.attempts {
            if a.stage == s.name && a.state == AttemptState::Complete {
                a.state = AttemptState::Cancelled {
                    reason: format!(
                        "{resource} was let go after it ran, so what it ran against may have been replaced by another ticket"
                    ),
                };
                a.ended_ms = Some(now_ms);
                back.get_or_insert(i);
            }
        }
    }
    back
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
    format!("{}/{}/before:{}:{}", t.id, rec.stage, rec.lane, rec.n)
}

/// What a `stuck` question carries as its "attempt": not an attempt of
/// any stage (`held_in` finds none, so it holds nothing), but the
/// record it is about.
fn stuck_key(rec: &ServiceRecord) -> (String, u32) {
    (format!("service:{}", rec.lane), rec.n)
}

fn is_stuck_for(d: &Decision, rec: &ServiceRecord) -> bool {
    d.name == STUCK && d.stage == rec.stage && d.attempt.as_ref() == Some(&stuck_key(rec))
}

/// The unacted answer to the record's `stuck` question, with its index.
fn stuck_answer(t: &Ticket, i: usize) -> Option<(usize, String)> {
    let rec = &t.services[i];
    t.decisions.iter().enumerate().find_map(|(d, x)| {
        x.unacted_answer()
            .filter(|_| is_stuck_for(x, rec))
            .map(|a| (d, a.to_owned()))
    })
}

fn stuck_pending(t: &Ticket, i: usize) -> bool {
    let rec = &t.services[i];
    t.decisions
        .iter()
        .any(|d| d.pending() && is_stuck_for(d, rec))
}

/// Each pending `stuck` question about the record withdrawn; true if
/// there was one.
fn withdraw_stuck(t: &mut Ticket, i: usize) -> bool {
    let rec = &t.services[i];
    let mut withdrawn = false;
    for d in &mut t.decisions {
        if d.pending() && is_stuck_for(d, rec) {
            d.state = DecisionState::Cancelled;
            withdrawn = true;
        }
    }
    withdrawn
}

/// The ports of a ticket's services not yet stopped.
fn live_ports(t: &Ticket) -> impl Iterator<Item = u16> + '_ {
    t.services
        .iter()
        .filter(|s| s.state != ServiceState::Stopped)
        .filter_map(|s| s.port)
}

/// The other tickets of `t`'s project that hold `resource`, as
/// `<id> (<source>)`.
fn holders_of(all: &[Ticket], t: &Ticket, resource: &str) -> Vec<String> {
    all.iter()
        .filter(|o| o.id != t.id && o.project == t.project)
        .filter(|o| !matches!(o.state, TicketState::Closed { .. }))
        .filter(|o| o.holds.iter().any(|h| h.resource == resource))
        .map(|o| format!("{} ({})", o.id, o.source.label()))
        .collect()
}

/// What an active ticket waits for: a resource its current stage needs
/// that as many other tickets hold as its `count` allows, and who holds
/// it, as `my-dev, held by baea8dbe (#56)`.
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
