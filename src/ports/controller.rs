//! A hand controller: a few buttons and a stick, read from a device so
//! the working set can be driven without a mouse. The events are what
//! the device reports; what they mean (move the selection, listen into
//! the selected session) is the core's business.

/// The two buttons of a Wii nunchuk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Z,
    C,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerEvent {
    Button {
        button: Button,
        down: bool,
    },
    /// The stick was flicked that way (or held there long enough to
    /// repeat). The device decides the dead zone and the repeat rate.
    Flick(Direction),
    /// The device appeared or went away.
    Connected(bool),
}

impl ControllerEvent {
    /// One line of the device's protocol: `Z1`, `C0`, `SU`, `SL`.
    /// `S0` (stick centred), `P` (a ping's answer), and comments are
    /// not events.
    #[must_use]
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim();
        let event = match line {
            "Z1" => Self::Button {
                button: Button::Z,
                down: true,
            },
            "Z0" => Self::Button {
                button: Button::Z,
                down: false,
            },
            "C1" => Self::Button {
                button: Button::C,
                down: true,
            },
            "C0" => Self::Button {
                button: Button::C,
                down: false,
            },
            "SU" => Self::Flick(Direction::Up),
            "SD" => Self::Flick(Direction::Down),
            "SL" => Self::Flick(Direction::Left),
            "SR" => Self::Flick(Direction::Right),
            _ => return None,
        };
        Some(event)
    }
}

/// The device, read from the poll tick like the event log. Nothing on
/// it blocks: a real adapter reads on its own thread.
pub trait Controller: Send {
    /// Events since the last poll, in order.
    fn poll(&mut self) -> Vec<ControllerEvent>;
    /// How many agents are waiting, for the device to show.
    fn set_waiting(&mut self, count: usize);
}
