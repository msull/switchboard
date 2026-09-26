//! The hand controller's meaning. The device reports buttons and stick
//! flicks; here they select a card on the working set being shown, open
//! a radial menu of actions on it, and hold a session open for
//! dictation. The selection is transient: a set with no selection
//! starts at its top-left card.

use super::action::{AppCore, Clock, Effect, Out, View};
use super::grid;
use super::model::{PinTarget, RecordId, SessionKind, SetId};
use crate::ports::controller::{Button, ControllerEvent, Direction};

/// The radial menu Z holds open on the selected card: a slice per
/// stick direction, the one the stick points at highlighted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadialMenu {
    pub target: RecordId,
    pub highlighted: Option<Direction>,
}

impl RadialMenu {
    /// What each slice does, by the direction that picks it.
    #[must_use]
    pub fn label(direction: Direction) -> &'static str {
        match direction {
            Direction::Up => "View",
            Direction::Right => "Open",
            Direction::Down => "Stop",
            Direction::Left => "Terminal",
        }
    }
}

/// Something the core wants shown that only the UI can show: the
/// shell moves these into the UI's state after a dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiRequest {
    /// The session's latest answer in the message dialog.
    ViewAnswer(RecordId),
    /// The session's live pane in a dialog.
    Terminal(RecordId),
}

#[derive(Debug, Default)]
pub(super) struct ControllerState {
    pub z: bool,
    pub c: bool,
    pub connected: bool,
    /// The selected card of each set the stick has moved on.
    pub active: Vec<(SetId, PinTarget)>,
    /// The session C is holding open for dictation.
    pub hold_listen: Option<RecordId>,
    pub menu: Option<RadialMenu>,
    pub requests: Vec<UiRequest>,
}

impl AppCore {
    pub(super) fn controller_event(&mut self, event: ControllerEvent, now: Clock, out: &mut Out) {
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
                    self.controller.menu = None;
                    self.info("Controller disconnected", now);
                }
            }
            ControllerEvent::Button {
                button: Button::Z,
                down: true,
            } => {
                self.controller.z = true;
                self.controller.menu = self.selected_session().map(|target| RadialMenu {
                    target,
                    highlighted: None,
                });
            }
            ControllerEvent::Button {
                button: Button::Z,
                down: false,
            } => {
                self.controller.z = false;
                if let Some(menu) = self.controller.menu.take()
                    && let Some(direction) = menu.highlighted
                {
                    self.radial_choice(menu.target, direction, now, out);
                }
            }
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
            ControllerEvent::Flick(direction) => match &mut self.controller.menu {
                Some(menu) => menu.highlighted = Some(direction),
                None => self.step_card(direction),
            },
            ControllerEvent::StickCentred => {
                if let Some(menu) = &mut self.controller.menu {
                    menu.highlighted = None;
                }
            }
        }
    }

    /// The radial menu's slice, let go of with the stick pointing at it.
    fn radial_choice(&mut self, id: RecordId, direction: Direction, now: Clock, out: &mut Out) {
        if self.session(id).is_none() {
            return;
        }
        match direction {
            Direction::Up => self.controller.requests.push(UiRequest::ViewAnswer(id)),
            Direction::Right => self.show_session(id, now, out),
            Direction::Down => self.aim_at_pane(id, out, |host| Effect::SendKeys {
                host,
                bytes: vec![0x1b],
            }),
            Direction::Left => self.controller.requests.push(UiRequest::Terminal(id)),
        }
    }

    /// The selected card of the working set being shown, if it is a
    /// session's.
    fn selected_session(&self) -> Option<RecordId> {
        let View::WorkingSet(set) = self.view() else {
            return None;
        };
        match self.active_card(set)? {
            PinTarget::Session(id) => Some(id),
            PinTarget::File(..) => None,
        }
    }

    /// The session C would hold open: the session being shown, else
    /// the selected card of the working set being shown if it is an
    /// agent's.
    fn listen_target(&self) -> Option<RecordId> {
        let id = match self.view() {
            View::Session(id) => id,
            View::WorkingSet(_) => self.selected_session()?,
            _ => return None,
        };
        let record = self.session(id)?;
        matches!(record.kind, SessionKind::Agent(_)).then_some(id)
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

    /// The radial menu while Z holds it open.
    #[must_use]
    pub fn radial_menu(&self) -> Option<&RadialMenu> {
        self.controller.menu.as_ref()
    }

    /// What the UI is asked to show, once each. The shell moves these
    /// into the UI's state after a dispatch.
    pub fn take_ui_requests(&mut self) -> Vec<UiRequest> {
        std::mem::take(&mut self.controller.requests)
    }

    #[must_use]
    pub fn controller_connected(&self) -> bool {
        self.controller.connected
    }
}
