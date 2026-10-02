//! The ledger against Switchboard, at start: every request whose reply
//! was never written is resolved from `find` and `op.status`, by its
//! class. Nothing is launched here; a lost creation is a decision.

use anyhow::Result;
use switchboard_control::{Body, Found, Made, OpStatus, Reply, Request};

use crate::scheduler::{Ask, Runner, apply_reply};
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

/// Whether recovery has nothing more to do for this operation: it has a
/// reply, or recovery already gave its verdict. Recovering a lost send
/// again would raise its decision again.
pub(crate) fn settled(op: &Operation) -> bool {
    op.reply.is_some() || op.settled
}

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
            let pending: Vec<usize> = t
                .ledger
                .iter()
                .enumerate()
                .filter(|(_, o)| o.unresolved())
                .map(|(i, _)| i)
                .collect();
            for i in pending {
                self.recover_one(&mut t, &mut ps, i, now_ms)?;
            }
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
                let found = match self.port.call(&Request::new(
                    format!("r-{}", uuid::Uuid::new_v4().simple()),
                    Body::Find {
                        operation: op.op.clone(),
                    },
                ))? {
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
            "idempotent" => self.replay(t, ps, i),
            _ => {
                give_verdict(&mut t.ledger[i], NOT_REPEATED);
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
            }
        }
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
        match self.port.call(&Request::new(op.op, body)) {
            Ok(reply) => {
                t.ledger[i].reply = Some(reply.clone());
                t.ledger[i].error = None;
                apply_reply(t, ps, &op.intent, &reply);
            }
            Err(e) => t.ledger[i].error = Some(e.to_string()),
        }
    }

    fn status_of(&mut self, op: &str) -> Result<OpStatus> {
        match self.port.call(&Request::new(
            format!("r-{}", uuid::Uuid::new_v4().simple()),
            Body::OpStatus {
                operation: op.to_owned(),
            },
        ))? {
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
