//! The hand controller's meaning. The device reports buttons and stick
//! flicks; here they select a card on the working set being shown and
//! hold a session open for dictation. The selection is transient: a
//! set with no selection starts at its top-left card.

use super::action::{AppCore, Clock, View};
use super::grid;
use super::model::{PinTarget, RecordId, SessionKind, SetId};
use crate::ports::controller::{Button, ControllerEvent, Direction};

#[derive(Debug, Default)]
pub(super) struct ControllerState {
    pub z: bool,
    pub c: bool,
    pub connected: bool,
    /// The selected card of each set the stick has moved on.
    pub active: Vec<(SetId, PinTarget)>,
    /// The session C is holding open for dictation.
    pub hold_listen: Option<RecordId>,
}

impl AppCore {
    pub(super) fn controller_event(&mut self, event: ControllerEvent, now: Clock) {
        match event {
            ControllerEvent::Connected(connected) => {
                self.controller.connected = connected;
                if connected {
                    self.info("Controller connected", now);
                } else {
                    // Nothing more will come from it: let go of everything.
                    self.controller.z = false;
                    self.controller.c = false;
                    self.controller.hold_listen = None;
                    self.info("Controller disconnected", now);
                }
            }
            ControllerEvent::Button {
                button: Button::Z,
                down,
            } => self.controller.z = down,
            ControllerEvent::Button {
                button: Button::C,
                down: true,
            } => {
                self.controller.c = true;
                match self.listen_target() {
                    Some(id) => self.controller.hold_listen = Some(id),
                    None => self.info("No agent selected to listen into", now),
                }
            }
            ControllerEvent::Button {
                button: Button::C,
                down: false,
            } => {
                self.controller.c = false;
                self.controller.hold_listen = None;
            }
            ControllerEvent::Flick(direction) => {
                // The stick only steers while Z is held, so brushing it
                // between dictations moves nothing.
                if self.controller.z {
                    self.step_card(direction);
                }
            }
        }
    }

    /// Select the card one step `direction` from the selected one, on
    /// the working set being shown. At the edge nothing moves.
    pub(super) fn step_card(&mut self, direction: Direction) {
        let View::WorkingSet(set) = self.view() else {
            return;
        };
        let Some(from) = self.active_card(set) else {
            return;
        };
        let next = self.working_set(set).and_then(|s| {
            let rect = s.items.iter().find(|i| i.target == from)?.rect;
            grid::neighbour(&s.items, rect, direction).cloned()
        });
        if let Some(target) = next {
            self.activate_card(set, target);
        }
    }

    /// The session C would hold open: the session being shown, else
    /// the selected card of the working set being shown if it is an
    /// agent's.
    fn listen_target(&self) -> Option<RecordId> {
        let id = match self.view() {
            View::Session(id) => id,
            View::WorkingSet(set) => match self.active_card(set)? {
                PinTarget::Session(id) => id,
                PinTarget::File(..) => return None,
            },
            _ => return None,
        };
        let record = self.session(id)?;
        matches!(record.kind, SessionKind::Agent(_)).then_some(id)
    }

    pub(super) fn activate_card(&mut self, set: SetId, target: PinTarget) {
        if !self
            .working_set(set)
            .is_some_and(|s| s.items.iter().any(|i| i.target == target))
        {
            return;
        }
        self.controller.active.retain(|(s, _)| *s != set);
        self.controller.active.push((set, target));
    }

    /// The selected card of `set`: the one the stick or a click chose
    /// while it is still there, else the top-left card.
    #[must_use]
    pub fn active_card(&self, set: SetId) -> Option<PinTarget> {
        let items = &self.working_set(set)?.items;
        let chosen = self
            .controller
            .active
            .iter()
            .find(|(s, _)| *s == set)
            .map(|(_, t)| t)
            .filter(|t| items.iter().any(|i| i.target == **t));
        match chosen {
            Some(target) => Some(target.clone()),
            None => items
                .iter()
                .min_by_key(|i| (i.rect.y, i.rect.x))
                .map(|i| i.target.clone()),
        }
    }

    /// The session the controller's C button is holding open for
    /// dictation, while it is held.
    #[must_use]
    pub fn hold_listen(&self) -> Option<RecordId> {
        self.controller.hold_listen
    }

    #[must_use]
    pub fn controller_connected(&self) -> bool {
        self.controller.connected
    }
}
