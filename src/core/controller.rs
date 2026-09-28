//! The hand controller's meaning. The device reports buttons and stick
//! flicks; here they select a card on the working set being shown, open
//! a radial menu of actions on it, and hold a session open for
//! dictation. The selection is transient: a set with no selection
//! starts at its top-left card.

use std::time::Duration;

use super::action::{AppCore, Clock, Effect, Out, View};
use super::grid;
use super::model::{PinTarget, RecordId, SessionKind, SetId};
use crate::ports::controller::{Button, ControllerEvent, Direction};

/// Two presses of C this close together latch listening on, so it
/// outlives the second press; two of Z are Escape.
pub const DOUBLE_PRESS: Duration = Duration::from_millis(400);

/// Holding the stick on a slice this long picks it, so the choice
/// does not depend on which of stick and Z is let go of first.
pub const DWELL: Duration = Duration::from_millis(500);

/// Where a radial menu opened, which decides its slices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    /// On the selected card of a working set.
    Card,
    /// On a session's own page.
    Session,
}

/// The radial menu Z holds open: a slice per stick direction, the one
/// the stick points at highlighted, picked by dwelling on it or by
/// letting Z go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadialMenu {
    pub target: RecordId,
    pub kind: MenuKind,
    pub highlighted: Option<Direction>,
    /// When the stick came to the highlighted slice.
    pub highlighted_since: Option<Duration>,
    /// How long it has been there, as of the last tick.
    pub held_for: Duration,
}

impl RadialMenu {
    /// What each slice does, by the direction that picks it.
    #[must_use]
    pub fn label(&self, direction: Direction) -> &'static str {
        match (self.kind, direction) {
            (MenuKind::Card, Direction::Up) => "View",
            (MenuKind::Card, Direction::Right) => "Open",
            (MenuKind::Session, Direction::Up) => "Pop out",
            (MenuKind::Session, Direction::Right) => "Back",
            (_, Direction::Down) => "Stop",
            (_, Direction::Left) => "Terminal",
        }
    }

    /// How far the dwell on the highlighted slice has come, 0 to 1.
    #[must_use]
    pub fn progress(&self) -> f32 {
        if self.highlighted.is_none() {
            return 0.0;
        }
        (self.held_for.as_secs_f32() / DWELL.as_secs_f32()).min(1.0)
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
    /// What the Escape key does: close a dialog, leave a text field.
    Escape,
    /// The raw pane under a session's conversation, shown or hidden.
    ToggleTerminal,
}

/// Which buttons are down. The device repeats both states once a
/// second as a heartbeat, so a line that changes nothing is not a
/// press or a release.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Held {
    pub z: bool,
    pub c: bool,
}

impl Held {
    fn get(self, button: Button) -> bool {
        match button {
            Button::Z => self.z,
            Button::C => self.c,
        }
    }

    fn set(&mut self, button: Button, down: bool) {
        match button {
            Button::Z => self.z = down,
            Button::C => self.c = down,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct ControllerState {
    pub held: Held,
    pub connected: bool,
    /// The selected card of each set the stick has moved on.
    pub active: Vec<(SetId, PinTarget)>,
    /// The session C is holding open for dictation.
    pub hold_listen: Option<RecordId>,
    /// How many C presses have started listening, so the UI can tell a
    /// fresh press from the same hold going on.
    pub listen_presses: u64,
    /// When C was last let go, for the double press.
    pub c_released_at: Option<Duration>,
    /// When Z was last let go, for the double press.
    pub z_released_at: Option<Duration>,
    /// The second press of a double press: letting go keeps listening.
    pub latched: bool,
    /// Where the stick is held, between a flick and its return.
    pub stick: Option<Direction>,
    /// The file card C is holding for the stick to scroll.
    pub scroll_hold: Option<PinTarget>,
    pub menu: Option<RadialMenu>,
    pub requests: Vec<UiRequest>,
}

impl AppCore {
    pub(super) fn controller_event(&mut self, event: ControllerEvent, now: Clock, out: &mut Out) {
        if let ControllerEvent::Button { button, down } = event {
            if self.controller.held.get(button) == down {
                return;
            }
            self.controller.held.set(button, down);
        }
        match event {
            ControllerEvent::Connected(connected) => {
                self.controller.connected = connected;
                if connected {
                    self.info("Controller connected", now);
                } else {
                    // Nothing more will come from it: let go of everything.
                    self.controller.held = Held::default();
                    self.controller.hold_listen = None;
                    self.controller.latched = false;
                    self.controller.scroll_hold = None;
                    self.controller.stick = None;
                    self.controller.menu = None;
                    self.info("Controller disconnected", now);
                }
            }
            ControllerEvent::Button {
                button: Button::Z,
                down: true,
            } => {
                // A second press within the window is Escape, and opens
                // no menu of its own.
                let double = self
                    .controller
                    .z_released_at
                    .is_some_and(|at| now.mono.saturating_sub(at) <= DOUBLE_PRESS);
                if double {
                    self.controller.requests.push(UiRequest::Escape);
                } else {
                    let opened = match self.view() {
                        View::Session(id) => Some((id, MenuKind::Session)),
                        View::WorkingSet(_) => {
                            self.selected_session().map(|id| (id, MenuKind::Card))
                        }
                        _ => None,
                    };
                    self.controller.menu = opened.map(|(target, kind)| RadialMenu {
                        target,
                        kind,
                        highlighted: None,
                        highlighted_since: None,
                        held_for: Duration::ZERO,
                    });
                }
            }
            ControllerEvent::Button {
                button: Button::Z,
                down: false,
            } => {
                self.controller.z_released_at = Some(now.mono);
                if let Some(menu) = self.controller.menu.take()
                    && let Some(direction) = menu.highlighted
                {
                    self.radial_choice(&menu, direction, now, out);
                }
            }
            ControllerEvent::Button {
                button: Button::C,
                down: true,
            } => self.c_down(now),
            ControllerEvent::Button {
                button: Button::C,
                down: false,
            } => {
                self.controller.c_released_at = Some(now.mono);
                self.controller.scroll_hold = None;
                if self.controller.latched {
                    self.controller.latched = false;
                } else {
                    self.controller.hold_listen = None;
                }
            }
            ControllerEvent::Flick(direction) => {
                self.controller.stick = Some(direction);
                if let Some(menu) = &mut self.controller.menu {
                    if menu.highlighted != Some(direction) {
                        menu.highlighted = Some(direction);
                        menu.highlighted_since = Some(now.mono);
                        menu.held_for = Duration::ZERO;
                    }
                } else if self.controller.scroll_hold.is_none() {
                    self.step_card(direction);
                }
            }
            ControllerEvent::StickCentred => {
                self.controller.stick = None;
                if let Some(menu) = &mut self.controller.menu {
                    menu.highlighted = None;
                    menu.highlighted_since = None;
                    menu.held_for = Duration::ZERO;
                }
            }
        }
    }

    /// C on a file card holds it for the stick to scroll. On a session
    /// it holds the session open for dictation; a second press within
    /// [`DOUBLE_PRESS`] latches that on, and the next press turns it
    /// off as a single press would.
    fn c_down(&mut self, now: Clock) {
        if let Some(target @ PinTarget::File(..)) = self.selected_card() {
            self.controller.scroll_hold = Some(target);
            return;
        }
        let double = self
            .controller
            .c_released_at
            .is_some_and(|at| now.mono.saturating_sub(at) <= DOUBLE_PRESS);
        match self.listen_target() {
            Some(id) => {
                self.controller.hold_listen = Some(id);
                self.controller.listen_presses += 1;
                if double {
                    self.controller.latched = true;
                    self.info("Listening stays on; press C to stop", now);
                }
            }
            None => self.info("No agent selected to listen into", now),
        }
    }

    /// The dwell: on every tick, a slice the stick has stayed on for
    /// [`DWELL`] is picked and the menu closes, Z still held or not.
    pub(super) fn controller_tick(&mut self, now: Clock, out: &mut Out) {
        let Some(menu) = &mut self.controller.menu else {
            return;
        };
        let (Some(direction), Some(since)) = (menu.highlighted, menu.highlighted_since) else {
            return;
        };
        menu.held_for = now.mono.saturating_sub(since);
        if menu.held_for >= DWELL {
            let menu = self.controller.menu.take().expect("the menu is open");
            self.radial_choice(&menu, direction, now, out);
        }
    }

    /// The radial menu's slice, picked with the stick pointing at it.
    fn radial_choice(
        &mut self,
        menu: &RadialMenu,
        direction: Direction,
        now: Clock,
        out: &mut Out,
    ) {
        let id = menu.target;
        if self.session(id).is_none() {
            return;
        }
        match (menu.kind, direction) {
            (MenuKind::Card, Direction::Up) => {
                self.controller.requests.push(UiRequest::ViewAnswer(id));
            }
            (MenuKind::Card, Direction::Right) => self.show_session(id, now, out),
            (MenuKind::Card, Direction::Left) => {
                self.controller.requests.push(UiRequest::Terminal(id));
            }
            (MenuKind::Session, Direction::Up) => self.pop_out(id, out),
            (MenuKind::Session, Direction::Right) => drop(self.view_stack.pop()),
            (MenuKind::Session, Direction::Left) => {
                self.controller.requests.push(UiRequest::ToggleTerminal);
            }
            (_, Direction::Down) => self.aim_at_pane(id, out, |host| Effect::SendKeys {
                host,
                bytes: vec![0x1b],
            }),
        }
    }

    /// The selected card of the working set being shown.
    fn selected_card(&self) -> Option<PinTarget> {
        let View::WorkingSet(set) = self.view() else {
            return None;
        };
        self.active_card(set)
    }

    /// The selected card of the working set being shown, if it is a
    /// session's.
    fn selected_session(&self) -> Option<RecordId> {
        match self.selected_card()? {
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

    /// How many times C has started listening. A new number with the
    /// same session is a fresh press, to be listened to again.
    #[must_use]
    pub fn listen_presses(&self) -> u64 {
        self.controller.listen_presses
    }

    /// The file card C is holding, and where the stick points, for
    /// the UI to scroll it.
    #[must_use]
    pub fn scroll_hold(&self) -> Option<(&PinTarget, Option<Direction>)> {
        self.controller
            .scroll_hold
            .as_ref()
            .map(|t| (t, self.controller.stick))
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
