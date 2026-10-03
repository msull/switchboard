//! The queue as a working set in Switchboard: one card per ticket, in
//! queue order, redrawn whole with `set.sync` whenever what it should
//! show changes. The order of record is the queue on Dispatch's side.

use anyhow::{Result, bail};
use switchboard_control::{Body, Pin, PinTarget, Rect, Reply};

use crate::scheduler::Runner;
use crate::ticket::{ProjectState, Ticket, TicketState};

/// A card's size in grid units.
const CARD: (u32, u32) = (10, 8);

/// Make the set once, then keep it showing the current session of every
/// ticket in flight or waiting, top to bottom in queue order. The
/// request goes on `owner`'s ledger when one is given (a closing ticket
/// clearing its own card), else on the one `ledger_ticket` picks.
pub fn sync_queue(
    runner: &mut Runner,
    ps: &mut ProjectState,
    project: &str,
    tickets: &[Ticket],
    owner: Option<&mut Ticket>,
    now_ms: u64,
) -> Result<()> {
    let shown: Vec<(String, String)> = ps
        .queue
        .iter()
        .filter_map(|id| tickets.iter().find(|t| &t.id == id))
        .filter(|t| {
            !matches!(
                t.state,
                TicketState::Closing { .. } | TicketState::Closed { .. }
            )
        })
        .filter_map(|t| t.current_session().map(|s| (t.id.clone(), s.clone())))
        .collect();
    if shown == ps.shown && ps.set.is_some() {
        return Ok(());
    }
    if shown.is_empty() && ps.set.is_none() {
        return Ok(());
    }
    // A set is made in the space, so without one there is no set yet.
    let Some(space) = ps.space.clone() else {
        return Ok(());
    };
    let mut found = match owner {
        Some(_) => None,
        None => ledger_ticket(runner, ps, &shown, tickets)?,
    };
    let t: &mut Ticket = match (owner, found.as_mut()) {
        (Some(owner), _) => owner,
        (None, Some(t)) => t,
        (None, None) => return Ok(()),
    };
    if ps.set.is_none() {
        // The queue view is not an attempt's, so the request names none.
        let reply = runner.send(
            t,
            ps,
            None,
            "set",
            Body::SetNew {
                space,
                name: format!("Dispatch · {project}"),
            },
            now_ms,
        )?;
        if ps.set.is_none() {
            bail!("set.new answered {reply:?}");
        }
    }
    let Some(set) = ps.set.clone() else {
        return Ok(());
    };
    let items: Vec<Pin> = shown
        .iter()
        .enumerate()
        .map(|(i, (_, session))| Pin {
            target: PinTarget::Session {
                session: session.clone(),
            },
            rect: Rect {
                x: 0,
                y: u32::try_from(i).unwrap_or(u32::MAX).saturating_mul(CARD.1),
                w: CARD.0,
                h: CARD.1,
            },
        })
        .collect();
    let reply = runner.send(t, ps, None, "sync", Body::SetSync { set, items }, now_ms)?;
    if let Reply::Failed { reason } = reply {
        bail!("set.sync: {reason}");
    }
    ps.shown = shown;
    Ok(())
}

/// The ticket a set request is written under when no closing ticket
/// brings its own: the first one shown, as the set was made under the
/// first ticket's ledger, else one whose card the set still holds
/// though it no longer has a session to show. `None` when every card
/// left belongs to a closing ticket: each clears its own when its close
/// gets there. A ticket nobody can read is an error, never a sync
/// pretended. `send` saves the ledger it writes, so the copy returned
/// needs no save of its own.
fn ledger_ticket(
    runner: &Runner,
    ps: &ProjectState,
    shown: &[(String, String)],
    tickets: &[Ticket],
) -> Result<Option<Ticket>> {
    if let Some(t) = tickets
        .iter()
        .find(|t| shown.iter().any(|(id, _)| id == &t.id))
    {
        return Ok(Some(t.clone()));
    }
    let mut closing = false;
    let mut unreadable = Vec::new();
    for (id, _) in &ps.shown {
        match runner.load_ticket(id) {
            Ok(t) if matches!(t.state, TicketState::Closing { .. }) => closing = true,
            Ok(t) => return Ok(Some(t)),
            Err(e) => unreadable.push(format!("{id}: {e:#}")),
        }
    }
    if closing {
        return Ok(None);
    }
    if unreadable.is_empty() {
        bail!("nothing shown and no ticket to sync the set under");
    }
    bail!(
        "nothing shown, and the set's tickets cannot be read to sync it under: {}",
        unreadable.join("; ")
    )
}
