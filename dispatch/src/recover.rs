//! The ledger against Switchboard, at start: every request whose reply
//! was never written is resolved from `find` and `op.status`, by its
//! class. Nothing is launched here; a lost creation is a decision.

use anyhow::Result;
use switchboard_control::{Body, Found, Made, OpStatus, Reply, Request};

use crate::scheduler::{Ask, NUDGE, Runner, apply_reply};
use crate::ticket::{DecisionKind, Operation, Ticket, TicketState};

/// Recovery's verdicts on an operation with no reply, as the reader
/// sees them. Whether an op is settled is its `settled` flag, not these
/// words; `store::migrate` matches them only to set that flag on
/// records written before it existed, so those copies must not change.
pub(crate) const LOST: &str = "lost: never reached Switchboard";
pub(crate) const INTERRUPTED: &str = "interrupted";
pub(crate) const REMOVED: &str = "removed by hand";
pub(crate) const NOT_REPEATED: &str = "reply lost; not repeated";
pub(crate) const HARMLESS: &str = "reply lost; harmless to repeat";
pub(crate) const SUPERSEDED: &str = "reply lost; a later request for the session replaced it";

/// Recovery's verdict written on an operation: the words for the
/// reader, and the flag that keeps a later pass from recovering it
/// again.
fn give_verdict(op: &mut Operation, verdict: &str) {
    op.error = Some(verdict.into());
    op.settled = true;
}

impl Runner {
    /// Resolve every unanswered operation of every ticket.
    pub fn recover(&mut self, now_ms: u64) -> Result<()> {
        self.transaction(|r| r.recover_locked(now_ms))
    }

    fn recover_locked(&mut self, now_ms: u64) -> Result<()> {
        for mut t in self.tickets()? {
            // A closing ticket's ledger is recovered by `finish_closing`,
            // which keeps the intent to close over whatever recovery
            // decides about an attempt.
            if matches!(
                t.state,
                TicketState::Closed { .. } | TicketState::Closing { .. }
            ) {
                continue;
            }
            let mut ps = self.load_project(&t.project)?;
            let pending = t.unsettled();
            self.health.borrow_mut().current = Some(t.id.clone());
            let recovered = pending
                .into_iter()
                .try_for_each(|i| self.recover_one(&mut t, &mut ps, i, now_ms));
            self.health.borrow_mut().current = None;
            recovered?;
            self.save_ticket(&mut t, now_ms)?;
            self.save_project(&ps)?;
        }
        Ok(())
    }

    /// One unanswered operation, resolved by its class.
    pub(crate) fn recover_one(
        &mut self,
        t: &mut Ticket,
        ps: &mut crate::ticket::ProjectState,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let op = t.ledger[i].clone();
        log::info!("ticket {} recovering {} ({})", t.id, op.op, op.kind);
        match op.class.as_str() {
            "creation" => {
                let found = match self.call(
                    Some(&t.id),
                    &Request::new(
                        format!("r-{}", uuid::Uuid::new_v4().simple()),
                        Body::Find {
                            operation: op.op.clone(),
                        },
                    ),
                )? {
                    Reply::Found { records } => records,
                    other => anyhow::bail!("find answered {other:?}"),
                };
                if found.is_empty() {
                    match self.status_of(&op.op)? {
                        OpStatus::InProgress => {}
                        OpStatus::Done { reply } => {
                            t.ledger[i].reply = Some(*reply.clone());
                            apply_reply(t, ps, &op.intent, &reply);
                        }
                        OpStatus::Unknown => {
                            give_verdict(&mut t.ledger[i], LOST);
                            self.fail_from_recovery(
                                t,
                                ps,
                                op.attempt.as_ref(),
                                "lost before it was sent",
                                now_ms,
                            )?;
                        }
                        OpStatus::Interrupted => {
                            give_verdict(&mut t.ledger[i], INTERRUPTED);
                            self.fail_from_recovery(
                                t,
                                ps,
                                op.attempt.as_ref(),
                                "Switchboard stopped while starting it",
                                now_ms,
                            )?;
                        }
                    }
                } else if found.iter().any(|f| f.removed) {
                    give_verdict(&mut t.ledger[i], REMOVED);
                    self.fail_from_recovery(
                        t,
                        ps,
                        op.attempt.as_ref(),
                        "removed by hand in the window",
                        now_ms,
                    )?;
                } else {
                    // Present proves the save, not the launch: the
                    // operation's status says whether Switchboard was
                    // still on it, or died in the middle.
                    match self.status_of(&op.op)? {
                        OpStatus::InProgress => {}
                        OpStatus::Interrupted => {
                            give_verdict(&mut t.ledger[i], INTERRUPTED);
                            self.fail_from_recovery(
                                t,
                                ps,
                                op.attempt.as_ref(),
                                "Switchboard stopped while starting it",
                                now_ms,
                            )?;
                        }
                        OpStatus::Unknown | OpStatus::Done { .. } => {
                            let reply = rebuild(&found);
                            t.ledger[i].reply = Some(reply.clone());
                            apply_reply(t, ps, &op.intent, &reply);
                        }
                    }
                }
            }
            "idempotent" if superseded(t, i) => give_verdict(&mut t.ledger[i], SUPERSEDED),
            "idempotent" => self.replay(t, ps, i),
            _ => self.not_repeated(t, ps, i, now_ms)?,
        }
        Ok(())
    }

    /// A non-replayable operation with its reply lost is never sent
    /// again. A nudge is on the attempt before it is sent, and one that
    /// never arrived goes unanswered, which the idle grace turns into the
    /// usual question; any other is asked about once.
    fn not_repeated(
        &mut self,
        t: &mut Ticket,
        ps: &mut crate::ticket::ProjectState,
        i: usize,
        now_ms: u64,
    ) -> Result<()> {
        let op = t.ledger[i].clone();
        give_verdict(&mut t.ledger[i], NOT_REPEATED);
        if op.intent == NUDGE {
            return Ok(());
        }
        // Asked once: the answer, not another pass, settles it.
        t.ledger[i].asked = true;
        let stage = op.attempt.as_ref().map_or("?", |(s, _)| s.as_str());
        self.ensure_decision(
            t,
            ps,
            Ask {
                stage,
                name: "lost-send",
                kind: DecisionKind::Permission,
                question: format!(
                    "The reply to {} ({}) was lost and it cannot be repeated safely. Look at the pane, then park or rerun.",
                    op.op, op.kind
                ),
                options: &["rerun", "park"],
                recommendation: None,
                attempt: op.attempt.clone(),
            },
            now_ms,
        )?;
        Ok(())
    }

    /// An idempotent operation is sent again as itself: Switchboard
    /// answers from its log if it ran, and runs it now if it did not,
    /// and either is right. A failure now leaves it for the next pass.
    pub(crate) fn replay(
        &mut self,
        t: &mut Ticket,
        ps: &mut crate::ticket::ProjectState,
        i: usize,
    ) {
        let op = t.ledger[i].clone();
        let Some(body) = op.body else {
            give_verdict(&mut t.ledger[i], HARMLESS);
            return;
        };
        match self.call(Some(&t.id), &Request::new(op.op, body)) {
            Ok(reply) => {
                t.ledger[i].reply = Some(reply.clone());
                t.ledger[i].error = None;
                apply_reply(t, ps, &op.intent, &reply);
            }
            Err(e) => t.ledger[i].error = Some(e.to_string()),
        }
    }

    fn status_of(&mut self, op: &str) -> Result<OpStatus> {
        match self.call(
            None,
            &Request::new(
                format!("r-{}", uuid::Uuid::new_v4().simple()),
                Body::OpStatus {
                    operation: op.to_owned(),
                },
            ),
        )? {
            Reply::OpStatus { status } => Ok(status),
            other => anyhow::bail!("op.status answered {other:?}"),
        }
    }

    fn fail_from_recovery(
        &mut self,
        t: &mut Ticket,
        ps: &mut crate::ticket::ProjectState,
        attempt: Option<&(String, u32)>,
        reason: &str,
        now_ms: u64,
    ) -> Result<()> {
        let Some((stage, n)) = attempt else {
            return Ok(());
        };
        // Only an open attempt is failed: one already ended (failed, or
        // cancelled by a park or a close) keeps the outcome it has.
        if t.attempts
            .iter()
            .any(|a| &a.stage == stage && a.n == *n && a.is_open())
        {
            self.fail_attempt(t, ps, stage, *n, reason, now_ms)?;
        }
        Ok(())
    }
}

/// A waiting request with no reply that a later waiting request for the
/// same session follows on the ledger. Whether it landed or not, the
/// later one decides the mark; sent again now it would land after that
/// one and undo it, so it is settled without being sent.
pub(crate) fn superseded(t: &Ticket, i: usize) -> bool {
    let session_of = |o: &Operation| match &o.body {
        Some(Body::SessionWaiting { session, .. }) => Some(session.clone()),
        _ => None,
    };
    let Some(session) = session_of(&t.ledger[i]) else {
        return false;
    };
    t.ledger[i + 1..]
        .iter()
        .any(|o| session_of(o).as_ref() == Some(&session))
}

/// The reply a creation would have carried, from what `find` reports.
fn rebuild(found: &[Found]) -> Reply {
    let made: Vec<Made> = found
        .iter()
        .map(|f| Made {
            kind: f.kind,
            id: f.id.clone(),
        })
        .collect();
    let launched = found.iter().any(|f| {
        f.session
            .as_ref()
            .is_some_and(|s| s.liveness != switchboard_control::Liveness::Missing)
    });
    if launched {
        Reply::Launched { made }
    } else {
        Reply::Persisted { made }
    }
}
