//! The queue as a working set in Switchboard: one card per ticket, in
//! queue order, redrawn whole with `set.sync` whenever what it should
//! show changes. The order of record is the queue on Dispatch's side.

use anyhow::{Result, bail};
use switchboard_control::{Body, Pin, PinTarget, Rect, Reply};

use crate::scheduler::Runner;
use crate::ticket::{ProjectState, Ticket, TicketState};

/// A card's size in grid units.
const CARD: (u32, u32) = (10, 8);

/// What a sync did. Each one means the set shows what it should.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Synced {
    /// `set.sync` went out and was answered.
    Sent,
    /// The set already shows exactly this, as its last reply said.
    Unchanged,
    /// The project never made a set, and has nothing to show.
    NoSet,
}

/// Make the set once, then keep it showing the current session of every
/// ticket in flight or waiting, top to bottom in queue order. The
/// request goes on `owner`'s ledger when one is given (a closing ticket
/// clearing its own card), else on the first shown ticket's; with
/// neither there is nothing to write it under, and that is an error,
/// never a sync pretended.
pub fn sync_queue(
    runner: &mut Runner,
    ps: &mut ProjectState,
    project: &str,
    tickets: &[Ticket],
    owner: Option<&mut Ticket>,
    now_ms: u64,
) -> Result<Synced> {
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
        return Ok(Synced::Unchanged);
    }
    if shown.is_empty() && ps.set.is_none() {
        return Ok(Synced::NoSet);
    }
    // A set is made in the space, so without one there is no set yet.
    let Some(space) = ps.space.clone() else {
        return Ok(Synced::NoSet);
    };
    // The set is made under the first ticket's ledger, like the space.
    let mut first = tickets
        .iter()
        .find(|t| shown.iter().any(|(id, _)| id == &t.id))
        .cloned();
    let t: &mut Ticket = match (owner, first.as_mut()) {
        (Some(owner), _) => owner,
        (None, Some(first)) => first,
        (None, None) => bail!("nothing shown and no closing ticket to sync the set under"),
    };
    if ps.set.is_none() {
        let reply = runner.send_for_view(
            t,
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
        return Ok(Synced::NoSet);
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
    let reply = runner.send_for_view(t, ps, "sync", Body::SetSync { set, items }, now_ms)?;
    if let Reply::Failed { reason } = reply {
        bail!("set.sync: {reason}");
    }
    ps.shown = shown;
    Ok(Synced::Sent)
}
