//! When the frame loop ticks and polls. Visible, every `logic` call ticks
//! and the host is listed once a second; hidden (eframe runs `logic` but
//! not `ui`), the tick backs off to `HIDDEN_TICK` so a covered window does
//! not fork `tmux` every second. A hook wake reads the event log at once
//! and lists the host at most once per `WAKE_MIN`, with one trailing list
//! for a burst. Pure: times are passed in, so the tests never sleep.

use std::time::{Duration, Instant};

/// How often the host is listed and the event log read while visible.
pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// How often the tick runs while the window is hidden.
pub const HIDDEN_TICK: Duration = Duration::from_secs(5);
/// The shortest gap between two host lists a hook wake may cause while
/// hidden; `PostToolUse` fires per tool call, so a wake can come often.
pub const WAKE_MIN: Duration = Duration::from_secs(1);

/// What `decide` remembers between calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cadence {
    last_tick: Option<Instant>,
    last_host: Option<Instant>,
    wake_pending: bool,
}

impl Cadence {
    /// Startup ran a tick's worth of work and listed the host at `now`.
    #[must_use]
    pub fn started(now: Instant) -> Self {
        Self {
            last_tick: Some(now),
            last_host: Some(now),
            wake_pending: false,
        }
    }

    /// The host was listed at `now` outside `decide` (a forced poll).
    #[must_use]
    pub fn host_polled(self, now: Instant) -> Self {
        Self {
            last_host: Some(now),
            ..self
        }
    }
}

/// What one `logic` call should do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    /// Dispatch the core tick and run the slower passes.
    pub tick: bool,
    /// List the host and look for Codex ids.
    pub host_poll: bool,
    /// Read the hook event log. Equal to `host_poll` when visible; a
    /// hidden wake reads the log without listing the host.
    pub events: bool,
    /// When to come back, for `request_repaint_after`.
    pub next_in: Duration,
}

/// Decide what this `logic` call runs. `visible` is whether `ui` ran since
/// the previous call; `woken` is whether a hook connected since then.
/// Due-ness comes from elapsed time, never from which repaint request
/// fired, because egui keeps only the smallest pending delay.
#[must_use]
pub fn decide(prev: Cadence, now: Instant, visible: bool, woken: bool) -> (Cadence, Step) {
    let since = |t: Option<Instant>| t.map_or(Duration::MAX, |t| now.saturating_duration_since(t));
    if visible {
        let host_poll = woken || since(prev.last_host) >= POLL_INTERVAL;
        let next = Cadence {
            last_tick: Some(now),
            last_host: if host_poll { Some(now) } else { prev.last_host },
            wake_pending: false,
        };
        let step = Step {
            tick: true,
            host_poll,
            events: host_poll,
            next_in: POLL_INTERVAL,
        };
        return (next, step);
    }
    let tick = since(prev.last_tick) >= HIDDEN_TICK;
    let wanted = woken || prev.wake_pending;
    let host_poll = tick || (wanted && since(prev.last_host) >= WAKE_MIN);
    let next = Cadence {
        last_tick: if tick { Some(now) } else { prev.last_tick },
        last_host: if host_poll { Some(now) } else { prev.last_host },
        wake_pending: wanted && !host_poll,
    };
    let to_tick = HIDDEN_TICK.saturating_sub(since(next.last_tick));
    let next_in = if next.wake_pending {
        to_tick.min(WAKE_MIN.saturating_sub(since(next.last_host)))
    } else {
        to_tick
    };
    let step = Step {
        tick,
        host_poll,
        events: tick || woken,
        next_in,
    };
    (next, step)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn visible_ticks_every_call_and_polls_each_second() {
        let t0 = Instant::now();
        let c = Cadence::started(t0);
        let (c, s) = decide(c, t0 + ms(500), true, false);
        assert!(s.tick);
        assert!(!s.host_poll && !s.events);
        assert_eq!(s.next_in, POLL_INTERVAL);
        let (c, s) = decide(c, t0 + ms(999), true, false);
        assert!(s.tick && !s.host_poll);
        let (c, s) = decide(c, t0 + ms(1000), true, false);
        assert!(s.tick && s.host_poll && s.events);
        let (_, s) = decide(c, t0 + ms(1100), true, true);
        assert!(s.host_poll && s.events, "a wake polls at once");
    }

    #[test]
    fn hidden_waits_five_seconds() {
        let t0 = Instant::now();
        let c = Cadence::started(t0);
        let (c, s) = decide(c, t0 + ms(4900), false, false);
        assert!(!s.tick && !s.host_poll && !s.events);
        assert_eq!(s.next_in, ms(100));
        let (_, s) = decide(c, t0 + ms(5000), false, false);
        assert!(s.tick && s.host_poll && s.events);
        assert_eq!(s.next_in, HIDDEN_TICK);
    }

    #[test]
    fn hidden_wake_reads_events_and_polls_when_a_second_has_passed() {
        let t0 = Instant::now();
        let c = Cadence::started(t0);
        let (c, s) = decide(c, t0 + ms(1500), false, true);
        assert!(!s.tick);
        assert!(s.events && s.host_poll);
        assert!(!c.wake_pending);
        assert_eq!(s.next_in, ms(3500));
    }

    #[test]
    fn hidden_wake_burst_lists_the_host_once_then_once_more() {
        let t0 = Instant::now();
        let mut c = Cadence::started(t0);
        let mut polls = 0;
        for i in 0..10 {
            let (next, s) = decide(c, t0 + ms(2000 + i * 90), false, true);
            assert!(s.events, "every wake reads the log");
            polls += usize::from(s.host_poll);
            c = next;
        }
        assert_eq!(polls, 1);
        assert!(c.wake_pending);
        // The last wake was at 2810 ms, the poll at 2000 ms: the window
        // ends at 3000 ms.
        let (c, s) = decide(c, t0 + ms(2810), false, false);
        assert!(!s.host_poll);
        assert_eq!(s.next_in, ms(190));
        let (c, s) = decide(c, t0 + ms(3000), false, false);
        assert!(s.host_poll && !s.events && !s.tick);
        assert!(!c.wake_pending);
        let (_, s) = decide(c, t0 + ms(3500), false, false);
        assert!(!s.host_poll, "one trailing poll, not more");
    }

    #[test]
    fn next_in_is_the_tick_unless_a_wake_is_pending() {
        let t0 = Instant::now();
        let c = Cadence::started(t0);
        let (c, s) = decide(c, t0 + ms(1000), false, false);
        assert_eq!(s.next_in, ms(4000));
        let (c, s) = decide(c, t0 + ms(1200), false, true);
        assert!(s.host_poll);
        assert_eq!(s.next_in, ms(3800));
        let (_, s) = decide(c, t0 + ms(1300), false, true);
        assert!(!s.host_poll);
        assert_eq!(s.next_in, ms(900));
    }

    #[test]
    fn uncovering_polls_the_host_on_the_first_visible_call() {
        let t0 = Instant::now();
        let c = Cadence::started(t0);
        let (c, _) = decide(c, t0 + ms(500), true, false);
        let (c, s) = decide(c, t0 + ms(5500), false, false);
        assert!(s.tick);
        let (c, s) = decide(c, t0 + ms(9000), false, false);
        assert!(!s.tick);
        let (_, s) = decide(c, t0 + ms(9100), true, false);
        assert!(s.tick && s.host_poll && s.events);
    }

    #[test]
    fn a_forced_poll_counts_as_a_host_list() {
        let t0 = Instant::now();
        let c = Cadence::default().host_polled(t0);
        let (_, s) = decide(c, t0 + ms(500), true, false);
        assert!(s.tick && !s.host_poll);
    }
}
