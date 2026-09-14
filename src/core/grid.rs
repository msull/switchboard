//! Placement on a working set's grid. Pure functions: the UI reports
//! how many columns fit and where a card was dropped; the rules live
//! here where they can be tested without a window.

use super::model::{GridRect, PinTarget, PinnedItem, SessionKind};

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
        (PinTarget::Session(_), Some(SessionKind::Command | SessionKind::Service)) => (7, 5),
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
