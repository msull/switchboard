//! The ledger against Switchboard, at start: every request whose reply
//! was never written is resolved from `find` and `op.status`, by its
//! class. Nothing is launched here; a lost creation is a decision.

use anyhow::Result;
use switchboard_control::{Body, Found, Made, OpStatus, Reply, Request};

use crate::scheduler::{Ask, Runner, apply_reply};
use crate::ticket::{AttemptState, DecisionKind, Ticket, TicketState};

impl Runner {
    /// Resolve every unanswered operation of every ticket.
    pub fn recover(&mut self, now_ms: u64) -> Result<()> {
        for mut t in self.tickets()? {
            if matches!(t.state, TicketState::Closed { .. }) {
                continue;
            }
            let mut ps = self.load_project(&t.project)?;
            let pending: Vec<usize> = t
                .ledger
                .iter()
                .enumerate()
                .filter(|(_, o)| o.reply.is_none())
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
                            t.ledger[i].error = Some("lost: never reached Switchboard".into());
                            self.fail_from_recovery(
                                t,
                                ps,
                                op.attempt.as_ref(),
                                "lost before it was sent",
                                now_ms,
                            )?;
                        }
                        OpStatus::Interrupted => {
                            t.ledger[i].error = Some("interrupted".into());
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
                    t.ledger[i].error = Some("removed by hand".into());
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
                            t.ledger[i].error = Some("interrupted".into());
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
            "idempotent" => {
                // The same op again is answered from Switchboard's log if
                // it ran, and runs now if it did not; either is right.
                t.ledger[i].error = Some("reply lost; harmless to repeat".into());
            }
            _ => {
                t.ledger[i].error = Some("reply lost; not repeated".into());
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
        if t.attempts.iter().any(|a| {
            &a.stage == stage && a.n == *n && !matches!(a.state, AttemptState::Failed { .. })
        }) {
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
