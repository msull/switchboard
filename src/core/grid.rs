//! Placement on a working set's grid. Pure functions: the UI reports
//! how many columns fit and where a card was dropped; the rules live
//! here where they can be tested without a window.

use super::model::{GridRect, PinTarget, PinnedItem, SessionKind};
use crate::ports::controller::Direction;

/// A card can be squeezed to this many units, and no fewer.
pub const MIN_WIDTH: u32 = 3;
/// And to this many units tall.
pub const MIN_HEIGHT: u32 = 2;

/// The size a target's card starts at. Today's board cards are 7 units
/// wide; working-set cards show more and get more room.
#[must_use]
pub fn default_size(target: &PinTarget, kind: Option<SessionKind>) -> (u32, u32) {
    match (target, kind) {
        (PinTarget::File(..), _) => (10, 10),
        (PinTarget::Session(_), Some(SessionKind::Command | SessionKind::Service)) => (10, 7),
        (PinTarget::Session(_), _) => (10, 8),
    }
}

/// The first spot, scanning row by row from the top left, where a card
/// `w` by `h` fits without touching another. A card wider than the
/// grid goes in the first column of the first free row.
#[must_use]
pub fn first_free(items: &[PinnedItem], w: u32, h: u32, columns: u32) -> GridRect {
    let columns = columns.max(MIN_WIDTH);
    let last_x = columns.saturating_sub(w);
    for y in 0.. {
        for x in 0..=last_x {
            let candidate = GridRect { x, y, w, h };
            if !items.iter().any(|i| i.rect.overlaps(candidate)) {
                return candidate;
            }
        }
    }
    unreachable!("the grid is unbounded downwards")
}

/// Keep `rect` on the grid: at least the minimum size, and never past
/// the left edge. The right edge is open; the view scrolls to it.
#[must_use]
pub fn clamp(rect: GridRect) -> GridRect {
    GridRect {
        w: rect.w.max(MIN_WIDTH),
        h: rect.h.max(MIN_HEIGHT),
        ..rect
    }
}

/// Whether `rect` can hold `target` without touching another card.
#[must_use]
pub fn fits(items: &[PinnedItem], target: &PinTarget, rect: GridRect) -> bool {
    !items
        .iter()
        .any(|i| i.target != *target && i.rect.overlaps(rect))
}

/// The card one step `direction` from `from`: the nearest card wholly
/// on that side, preferring one that shares rows (or columns) with
/// `from`, then the closest to its centre line. None at the edge: the
/// selection does not wrap.
#[must_use]
pub fn neighbour(items: &[PinnedItem], from: GridRect, direction: Direction) -> Option<&PinTarget> {
    // Rotate every rect so the step is always "towards larger main".
    let span = |r: GridRect| -> (u32, u32, u32, u32) {
        let (x0, x1, y0, y1) = (r.x, r.x + r.w, r.y, r.y + r.h);
        match direction {
            Direction::Right => (x0, x1, y0, y1),
            Direction::Left => (u32::MAX - x1, u32::MAX - x0, y0, y1),
            Direction::Down => (y0, y1, x0, x1),
            Direction::Up => (u32::MAX - y1, u32::MAX - y0, x0, x1),
        }
    };
    let (_, from_end, from_c0, from_c1) = span(from);
    let from_mid = from_c0 + from_c1;
    items
        .iter()
        .filter_map(|item| {
            let (start, _, c0, c1) = span(item.rect);
            if start < from_end {
                return None;
            }
            let shares = c0 < from_c1 && from_c0 < c1;
            let mid = c0 + c1;
            let off = mid.abs_diff(from_mid);
            Some(((!shares, start - from_end, off), &item.target))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, target)| target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::RecordId;

    fn item(x: u32, y: u32, w: u32, h: u32) -> PinnedItem {
        PinnedItem {
            target: PinTarget::Session(RecordId::new()),
            rect: GridRect { x, y, w, h },
        }
    }

    #[test]
    fn a_step_lands_on_the_nearest_card_that_shares_rows_and_stops_at_the_edge() {
        // Two rows: a wide card over two narrow ones, and one far right.
        //   [ a a a a ] [ d ]
        //   [ b ] [ c ]
        let items = vec![
            item(0, 0, 8, 2),
            item(0, 2, 4, 2),
            item(4, 2, 4, 2),
            item(8, 0, 4, 2),
        ];
        let (a, b, c, d) = (&items[0], &items[1], &items[2], &items[3]);
        let step = |from: &PinnedItem, dir| neighbour(&items, from.rect, dir);
        assert_eq!(step(a, Direction::Right), Some(&d.target));
        assert_eq!(
            step(a, Direction::Down),
            Some(&b.target),
            "the one nearest a's centre line"
        );
        assert_eq!(step(c, Direction::Up), Some(&a.target));
        assert_eq!(step(b, Direction::Right), Some(&c.target));
        assert_eq!(step(c, Direction::Left), Some(&b.target));
        // d shares no rows with b or c; it is still the only card to the right.
        assert_eq!(step(c, Direction::Right), Some(&d.target));
        assert_eq!(step(d, Direction::Right), None);
        assert_eq!(step(a, Direction::Up), None);
        assert_eq!(step(b, Direction::Left), None);
    }

    #[test]
    fn first_free_scans_rows_then_columns() {
        assert_eq!(
            first_free(&[], 10, 8, 24),
            GridRect {
                x: 0,
                y: 0,
                w: 10,
                h: 8
            }
        );
        let items = [item(0, 0, 10, 8)];
        assert_eq!(
            first_free(&items, 10, 8, 24),
            GridRect {
                x: 10,
                y: 0,
                w: 10,
                h: 8
            }
        );
        // Not enough room to the right: the next row.
        assert_eq!(
            first_free(&items, 20, 8, 24),
            GridRect {
                x: 0,
                y: 8,
                w: 20,
                h: 8
            }
        );
        // A gap between two cards is used when the card fits it.
        let items = [item(0, 0, 4, 4), item(8, 0, 4, 4)];
        assert_eq!(
            first_free(&items, 4, 4, 24),
            GridRect {
                x: 4,
                y: 0,
                w: 4,
                h: 4
            }
        );
        assert_eq!(
            first_free(&items, 5, 4, 24),
            GridRect {
                x: 12,
                y: 0,
                w: 5,
                h: 4
            }
        );
    }

    #[test]
    fn a_card_wider_than_the_grid_still_gets_a_row() {
        let items = [item(0, 0, 3, 3)];
        assert_eq!(
            first_free(&items, 30, 2, 12),
            GridRect {
                x: 0,
                y: 3,
                w: 30,
                h: 2
            }
        );
    }

    #[test]
    fn overlap_and_fit() {
        let a = GridRect {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
        };
        assert!(a.overlaps(GridRect {
            x: 3,
            y: 3,
            w: 2,
            h: 2
        }));
        assert!(!a.overlaps(GridRect {
            x: 4,
            y: 0,
            w: 2,
            h: 2
        }));
        let items = [item(0, 0, 4, 4)];
        let me = items[0].target.clone();
        // Moving over its own old place is fine; over another is not.
        assert!(fits(
            &items,
            &me,
            GridRect {
                x: 1,
                y: 1,
                w: 4,
                h: 4
            }
        ));
        let other = PinTarget::Session(RecordId::new());
        assert!(!fits(
            &items,
            &other,
            GridRect {
                x: 1,
                y: 1,
                w: 4,
                h: 4
            }
        ));
        assert!(fits(
            &items,
            &other,
            GridRect {
                x: 4,
                y: 0,
                w: 4,
                h: 4
            }
        ));
        assert_eq!(
            clamp(GridRect {
                x: 2,
                y: 0,
                w: 1,
                h: 1
            }),
            GridRect {
                x: 2,
                y: 0,
                w: MIN_WIDTH,
                h: MIN_HEIGHT
            }
        );
    }
}
