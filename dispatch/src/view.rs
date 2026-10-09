//! The queue as a working set in Switchboard: one card per ticket, in
//! queue order, redrawn whole with `set.sync` whenever what it should
//! show changes. The order of record is the queue on Dispatch's side.

use anyhow::{Result, bail};
use switchboard_control::{Body, Pin, PinTarget, RecordKind, Rect, Reply};

use crate::recover::ViewState;
use crate::scheduler::Runner;
use crate::ticket::{ProjectState, Ticket, TicketState};

/// A card's size in grid units.
const CARD: (u32, u32) = (10, 8);

/// Make the set once, then keep it showing the current session of every
/// ticket in flight or waiting, top to bottom in queue order. Every
/// request goes on the project's `view_op`, never a ticket's ledger, so
/// a queue change leaves every ticket's record alone. A request of an
/// earlier call whose reply never came is resolved first: a `set.new`
/// still in flight holds every new request back, and a lost `set.sync`
/// is superseded by a redraw of what the queue shows now.
pub fn sync_queue(
    runner: &mut Runner,
    ps: &mut ProjectState,
    project: &str,
    tickets: &[Ticket],
    now_ms: u64,
) -> Result<()> {
    let redraw = match runner.recover_view(ps)? {
        // An error, not a quiet return: a close takes `Ok` to mean its
        // card is off the set.
        ViewState::InFlight => bail!("a request for the queue set is still in flight"),
        ViewState::Redraw => true,
        ViewState::Settled => false,
    };
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
    if !redraw && shown == ps.shown && ps.set.is_some() {
        return Ok(());
    }
    // A superseded `set.sync` implies a set, so `redraw` never gets here.
    if shown.is_empty() && ps.set.is_none() {
        return Ok(());
    }
    // A set is made in the space, so without one there is no set yet.
    let Some(space) = ps.space.clone() else {
        return Ok(());
    };
    if ps.set.is_none() {
        let reply = runner.send_view(
            ps,
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
    let reply = runner.send_view(ps, "sync", Body::SetSync { set, items }, now_ms)?;
    if let Reply::Failed { reason } = reply {
        bail!("set.sync: {reason}");
    }
    ps.shown = shown;
    Ok(())
}

/// A queue-set reply applied to the project by the op's intent: a
/// `set.new` names the set. Recovery uses this too, with a reply rebuilt
/// from `find`.
pub(crate) fn apply_view_reply(ps: &mut ProjectState, intent: &str, reply: &Reply) {
    if intent == "set"
        && let Some(made) = reply.made().iter().find(|m| m.kind == RecordKind::Set)
    {
        ps.set = Some(made.id.clone());
    }
}
