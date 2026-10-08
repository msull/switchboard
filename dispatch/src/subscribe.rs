//! A supervisor's subscriptions: the tickets whose events the runner
//! types into the supervisor's pane, so an idle supervisor hears of a
//! move without a watch of its own.
//!
//! The record is the project's (`Supervision`), not the ticket's: a
//! cursor moved on every delivery would otherwise move the ticket's
//! `updated_ms`, which `wait` reads, and a Fresh inherits them as they
//! are. Delivery always goes to `supervisor.current`, through
//! Switchboard's `session.prompt`, which types only into a pane that is
//! ready for a prompt and checks that in the same step. The cursor is
//! the queue: a delivery the pane is not ready for waits on the record
//! and is offered again on the next pass. A delivery is written down
//! before it is sent and sent again under the same op after a lost
//! reply, which the app answers from its log without typing twice.

use anyhow::{Result, bail};
use switchboard_control::{self as wire, Body, Liveness, Reply, Request};

use crate::UsageError;
use crate::events::{self, Event, Kind};
use crate::scheduler::Runner;
use crate::store::read_ticket;
use crate::ticket::{Delivery, ProjectState, Subscription, Supervision, TicketState};

/// A delivery stops taking tickets once it holds this many lines; the
/// rest goes in on a later pass.
pub const DELIVERY_MAX_LINES: usize = 30;

/// How long the runner waits before trying `session.prompt` again on an
/// app that did not know it.
pub const UNSUPPORTED_RETRY_MS: u64 = 10 * 60 * 1000;

/// The first words of every delivery, which the seed names so the
/// supervisor tells a delivery from the owner's prompts.
pub const DELIVERY_HEADER: &str = "Dispatch subscription";

/// `delivery_waits` for an app that answered `session.prompt` as a bad
/// request: it never parsed it, so nothing was typed.
pub const UPDATE_THE_APP: &str = "Switchboard does not know session.prompt; update the app";

/// Whether a line of kind `kind` is one a subscription delivers: what
/// `wait --for move` returns, and the park or close that ends it.
#[must_use]
pub fn delivered(kind: Kind) -> bool {
    events::For::Move.candidate(kind) || matches!(kind, Kind::Parked | Kind::Closed)
}

/// The lines of `sub`'s ticket after its cursor that a delivery would
/// carry, not withdrawn, oldest first: what has not reached the
/// supervisor yet.
pub fn undelivered(data: &crate::store::DataDir, sub: &Subscription) -> Result<Vec<Event>> {
    let all = events::read_since(&events::log_path(data), sub.since)?;
    let gone = events::withdrawn(&all);
    Ok(all
        .into_iter()
        .filter(|e| e.ticket == sub.ticket && delivered(e.kind) && !gone.contains(&e.seq))
        .collect())
}

/// Set why deliveries wait; whether it changed, so the caller writes
/// and logs only a change.
fn set_waits(sup: &mut Supervision, waits: Option<String>, now_ms: u64) -> bool {
    if sup.delivery_waits == waits {
        return false;
    }
    sup.delivery_waits = waits;
    sup.delivery_waits_ms = now_ms;
    true
}

/// What one pass would deliver: the text and how it moves the cursors.
struct Due {
    text: String,
    through: Vec<(String, u64)>,
    closes: Vec<String>,
    parks: Vec<String>,
}

impl Runner {
    /// Follow `ticket` from the log's tail, or from `since`; subscribing
    /// again moves the cursor only when `since` is given. A closed
    /// ticket is refused. A standing park is marked seen, so it is not
    /// delivered as news.
    pub fn subscribe(
        &mut self,
        ticket: &str,
        what: &str,
        since: Option<u64>,
        by: &str,
        now_ms: u64,
    ) -> Result<Subscription> {
        if events::For::parse(what) != Some(events::For::Move) {
            return Err(UsageError("--for takes move".to_owned()).into());
        }
        self.transaction(|r| {
            let t = r.load_ticket(ticket)?;
            if matches!(
                t.state,
                TicketState::Closed { .. } | TicketState::Closing { .. }
            ) {
                bail!("ticket {ticket} is closed; there is nothing to follow");
            }
            let mut ps = r.load_project(&t.project)?;
            let subs = &mut ps.supervisor.subscriptions;
            let sub = if let Some(sub) = subs.iter_mut().find(|s| s.ticket == ticket) {
                if let Some(since) = since {
                    sub.since = since;
                }
                sub.clone()
            } else {
                let since = match since {
                    Some(since) => since,
                    None => events::last_seq(&events::log_path(&r.data))?,
                };
                let sub = Subscription {
                    ticket: ticket.to_owned(),
                    what: what.to_owned(),
                    since,
                    by: by.to_owned(),
                    at_ms: now_ms,
                    park_seen: matches!(t.state, TicketState::Parked { .. }),
                };
                subs.push(sub.clone());
                sub
            };
            r.save_project(&ps)?;
            Ok(sub)
        })
    }

    /// Stop following `ticket`; whether it was followed.
    pub fn unsubscribe(&mut self, ticket: &str) -> Result<bool> {
        self.transaction(|r| {
            let t = r.load_ticket(ticket)?;
            let mut ps = r.load_project(&t.project)?;
            let before = ps.supervisor.subscriptions.len();
            ps.supervisor.subscriptions.retain(|s| s.ticket != ticket);
            let gone = ps.supervisor.subscriptions.len() != before;
            if gone {
                r.save_project(&ps)?;
            }
            Ok(gone)
        })
    }

    /// Type each subscribed ticket's settled burst into the supervisor's
    /// pane, once it is ready for a prompt. A delivery left on the record
    /// is settled before anything new is offered. Nothing is resumed or
    /// launched, and a pass with nothing due writes nothing and asks
    /// Switchboard nothing.
    pub fn deliver_subscriptions(&mut self, project: &str, now_ms: u64) -> Result<()> {
        let ps = self.load_project(project)?;
        let sup = &ps.supervisor;
        if sup.subscriptions.is_empty() && sup.delivery.is_none() {
            return Ok(());
        }
        let Some(current) = sup.current.as_ref().map(|c| c.session.clone()) else {
            return Ok(());
        };
        if let Some(d) = sup.delivery.clone() {
            if d.session == current {
                return self.send_delivery(project, &d, now_ms);
            }
            // The supervisor was replaced with this in flight. A resend
            // could type into a past session the owner resumed by hand,
            // and its reply could not change the outcome; the cursors
            // stay, so the new supervisor gets the same lines.
            log::info!(
                "{project}: delivery {} to {} dropped: the supervisor was replaced",
                d.op,
                d.session
            );
            return self.transaction(|r| {
                let mut ps = r.load_project(project)?;
                if ps
                    .supervisor
                    .delivery
                    .as_ref()
                    .is_some_and(|x| x.op == d.op)
                {
                    ps.supervisor.delivery = None;
                    r.save_project(&ps)?;
                }
                Ok(())
            });
        }
        if sup.delivery_waits.as_deref() == Some(UPDATE_THE_APP)
            && now_ms < sup.delivery_waits_ms.saturating_add(UNSUPPORTED_RETRY_MS)
        {
            return Ok(());
        }
        let (due, unparked) = self.due(&ps, now_ms);
        let Some(due) = due else {
            if !unparked.is_empty() {
                self.settle_project(project, |sup| {
                    unpark(sup, &unparked);
                    true
                })?;
            }
            return Ok(());
        };
        // Asked first, and not logged by the app: a supervisor at work
        // costs one query a pass and no write in either program.
        let refusal = match self.session_view(&current)? {
            Err(reason) => Some(reason),
            Ok(view) if view.liveness != Liveness::Running => Some("not running".to_owned()),
            Ok(view) => view.prompt_refusal,
        };
        if let Some(reason) = refusal {
            return self.settle_project(project, |sup| {
                let changed = set_waits(sup, Some(reason.clone()), now_ms);
                if changed {
                    log::info!("{project}: a delivery waits: {reason}");
                }
                unpark(sup, &unparked) || changed
            });
        }
        let d = Delivery {
            op: format!("sub-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
            session: current,
            text: due.text,
            through: due.through,
            closes: due.closes,
            parks: due.parks,
            at_ms: now_ms,
        };
        self.settle_project(project, |sup| {
            unpark(sup, &unparked);
            sup.delivery = Some(d.clone());
            true
        })?;
        self.send_delivery(project, &d, now_ms)
    }

    /// What is due across the subscriptions, and the tickets whose
    /// `park_seen` no longer holds because they are not parked. A ticket
    /// that cannot be read is skipped and logged; the others still go.
    fn due(&self, ps: &ProjectState, now_ms: u64) -> (Option<Due>, Vec<String>) {
        let project = &ps.name;
        let decide = std::env::current_exe().map_or_else(
            |_| "dispatch decide".to_owned(),
            |exe| format!("{} decide", exe.display()),
        );
        let mut unparked = Vec::new();
        let mut body: Vec<String> = Vec::new();
        let mut due = Due {
            text: String::new(),
            through: Vec::new(),
            closes: Vec::new(),
            parks: Vec::new(),
        };
        for sub in &ps.supervisor.subscriptions {
            let path = self.data.ticket_file(&sub.ticket);
            let parked = match read_ticket(&path) {
                Ok(t) => matches!(t.state, TicketState::Parked { .. }),
                Err(e) => {
                    log::warn!("{project}: subscription to {}: {e:#}", sub.ticket);
                    continue;
                }
            };
            if sub.park_seen && !parked {
                unparked.push(sub.ticket.clone());
            }
            if body.len() >= DELIVERY_MAX_LINES {
                continue;
            }
            let park_seen = sub.park_seen && parked;
            let lines =
                match events::next_burst(&self.data, &sub.ticket, sub.since, park_seen, now_ms) {
                    Ok(Some(lines)) if !lines.is_empty() => lines,
                    Ok(_) => continue,
                    Err(e) => {
                        log::warn!("{project}: subscription to {}: {e:#}", sub.ticket);
                        continue;
                    }
                };
            for e in &lines {
                body.push(events::line(e, false));
            }
            // Read again for the hint: a decision answered since its line
            // has no answer left to give.
            if let Some(last) = lines.last()
                && last.kind == Kind::Decision
                && let Ok(t) = read_ticket(&path)
                && let Some(hint) = events::decide_hint(last, &t, &decide)
            {
                body.push(hint);
            }
            let top = lines.iter().map(|e| e.seq).max().unwrap_or(0);
            due.through.push((sub.ticket.clone(), top.max(sub.since)));
            if lines.iter().any(|e| e.kind == Kind::Closed) {
                due.closes.push(sub.ticket.clone());
            }
            if lines.iter().any(|e| e.kind == Kind::Parked) {
                due.parks.push(sub.ticket.clone());
            }
        }
        if body.is_empty() {
            return (None, unparked);
        }
        let tickets: Vec<&str> = due.through.iter().map(|(t, _)| t.as_str()).collect();
        due.text = format!(
            "{DELIVERY_HEADER}: events on {}\n{}\nAct on these as your seed says; nothing to re-arm.",
            tickets.join(", "),
            body.join("\n")
        );
        (Some(due), unparked)
    }

    /// Send `d` under its own op and settle it from the reply. A lost
    /// reply, or the port's `NO_ANSWER`, leaves it for the next pass to
    /// send again; the app answers a repeat from its log.
    fn send_delivery(&mut self, project: &str, d: &Delivery, now_ms: u64) -> Result<()> {
        let request = Request::new(
            d.op.clone(),
            Body::SessionPrompt {
                session: d.session.clone(),
                text: d.text.clone(),
            },
        );
        let reply = match self.call(None, &request) {
            Ok(reply) if !reply.is_no_answer() => reply,
            Ok(_) => {
                log::warn!(
                    "{project}: delivery {}: {}; sent again",
                    d.op,
                    wire::NO_ANSWER
                );
                return Ok(());
            }
            Err(e) => {
                log::warn!("{project}: delivery {}: {e}; sent again", d.op);
                return Ok(());
            }
        };
        self.settle_project(project, |sup| {
            if sup.delivery.as_ref().is_none_or(|x| x.op != d.op) {
                return false;
            }
            sup.delivery = None;
            if let Reply::Failed { reason } = &reply {
                let waits = if reason.starts_with("bad request") {
                    UPDATE_THE_APP.to_owned()
                } else {
                    reason.clone()
                };
                if set_waits(sup, Some(waits.clone()), now_ms) {
                    log::info!("{project}: a delivery waits: {waits}");
                } else if waits == UPDATE_THE_APP {
                    // The retry is throttled from the latest refusal, not
                    // the first, or every pass after one retry sends again.
                    sup.delivery_waits_ms = now_ms;
                }
                return true;
            }
            for sub in &mut sup.subscriptions {
                if let Some((_, seq)) = d.through.iter().find(|(t, _)| *t == sub.ticket) {
                    sub.since = sub.since.max(*seq);
                }
                if d.parks.contains(&sub.ticket) {
                    sub.park_seen = true;
                }
            }
            // Closing ends a subscription once the close is delivered.
            sup.subscriptions.retain(|s| !d.closes.contains(&s.ticket));
            set_waits(sup, None, now_ms);
            log::info!("{project}: delivered {} to {}", d.op, d.session);
            true
        })
    }

    /// Change the project's supervision under the writer lock, saving it
    /// when `f` says it changed.
    fn settle_project(
        &mut self,
        project: &str,
        f: impl FnOnce(&mut Supervision) -> bool,
    ) -> Result<()> {
        self.transaction(|r| {
            let mut ps = r.load_project(project)?;
            if f(&mut ps.supervisor) {
                r.save_project(&ps)?;
            }
            Ok(())
        })
    }
}

/// Clear `park_seen` on `tickets`; whether any was set.
fn unpark(sup: &mut Supervision, tickets: &[String]) -> bool {
    let mut changed = false;
    for sub in &mut sup.subscriptions {
        if sub.park_seen && tickets.contains(&sub.ticket) {
            sub.park_seen = false;
            changed = true;
        }
    }
    changed
}
