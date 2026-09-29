//! The queue as a working set in Switchboard: one card per ticket, in
//! queue order, redrawn whole with `set.sync` whenever what it should
//! show changes. The order of record is the queue on Dispatch's side.

use anyhow::Result;
use switchboard_control::{Body, Pin, PinTarget, Rect, Reply};

use crate::pipeline::Pipeline;
use crate::scheduler::Runner;
use crate::ticket::{ProjectState, Ticket, TicketState};

/// A card's size in grid units.
const CARD: (u32, u32) = (10, 8);

/// Make the set once, then keep it showing the current session of every
/// ticket in flight or waiting, top to bottom in queue order.
pub fn sync_queue(
    runner: &mut Runner,
    ps: &mut ProjectState,
    p: &Pipeline,
    tickets: &[Ticket],
    now_ms: u64,
) -> Result<()> {
    let shown: Vec<(String, String)> = ps
        .queue
        .iter()
        .filter_map(|id| tickets.iter().find(|t| &t.id == id))
        .filter(|t| !matches!(t.state, TicketState::Closed { .. }))
        .filter_map(|t| t.current_session().map(|s| (t.id.clone(), s.clone())))
        .collect();
    if shown == ps.shown && ps.set.is_some() {
        return Ok(());
    }
    if shown.is_empty() && ps.set.is_none() {
        return Ok(());
    }
    let Some(space) = ps.space.clone() else {
        return Ok(());
    };
    // The set is made under the first ticket's ledger, like the space.
    let Some(first) = tickets
        .iter()
        .find(|t| shown.iter().any(|(id, _)| id == &t.id))
        .cloned()
    else {
        return Ok(());
    };
    let mut t = first;
    if ps.set.is_none() {
        let reply = runner.send_for_view(
            &mut t,
            ps,
            "set",
            Body::SetNew {
                space,
                name: format!("Dispatch · {}", p.project.name),
            },
            now_ms,
        )?;
        if ps.set.is_none() {
            anyhow::bail!("set.new answered {reply:?}");
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
    let reply = runner.send_for_view(&mut t, ps, "sync", Body::SetSync { set, items }, now_ms)?;
    if let Reply::Failed { reason } = reply {
        anyhow::bail!("set.sync: {reason}");
    }
    ps.shown = shown;
    Ok(())
}
