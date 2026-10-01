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
/// clearing its own card), else on the first shown ticket's, else on
/// that of a ticket the set still shows; with none there is nothing to
/// write it under, and that is an error, never a sync pretended.
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
    // The set is made under the first ticket's ledger, like the space.
    let mut first = tickets
        .iter()
        .find(|t| shown.iter().any(|(id, _)| id == &t.id))
        .cloned();
    // Nothing to show any more, but a card is still up (a close stopped
    // between clearing its card and saving the project, or one from
    // before closes cleared their own): the set is emptied under the
    // ledger of a ticket it still shows, which is saved with the reply.
    let mut stale = None;
    // Why a shown ticket could not be read: the real cause when no
    // other ticket can carry the sync.
    let mut unreadable = Vec::new();
    if owner.is_none() && first.is_none() {
        for (id, _) in &ps.shown {
            match runner.load_ticket(id) {
                // A closing ticket clears its own card under its own
                // ledger.
                Ok(t) if matches!(t.state, TicketState::Closing { .. }) => {}
                Ok(t) => {
                    stale = Some(t);
                    break;
                }
                Err(e) => unreadable.push(format!("{id}: {e:#}")),
            }
        }
    }
    let t: &mut Ticket = match (owner, first.as_mut(), stale.as_mut()) {
        (Some(owner), _, _) => owner,
        (None, Some(first), _) => first,
        (None, None, Some(stale)) => stale,
        (None, None, None) if unreadable.is_empty() => {
            bail!("nothing shown and no ticket to sync the set under")
        }
        (None, None, None) => bail!(
            "nothing shown, and the set's tickets cannot be read to sync it under: {}",
            unreadable.join("; ")
        ),
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
    let reply = runner.send_for_view(t, ps, "sync", Body::SetSync { set, items }, now_ms)?;
    if let Some(stale) = stale.as_mut() {
        runner.save_ticket(stale, now_ms)?;
    }
    if let Reply::Failed { reason } = reply {
        bail!("set.sync: {reason}");
    }
    ps.shown = shown;
    Ok(())
}
