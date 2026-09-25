# Spike 10: a nunchuk over USB serial

Question: can a Wii nunchuk on a Feather RP2040 drive the working set
(select a card, hold a session open for dictation) fast enough to feel
direct, through a port the app owns, and without the app caring when
the cable comes and goes?

Mechanism: the Feather runs CircuitPython (`firmware/nunchuk/code.py`)
and prints one short line per change over USB CDC: `Z1`/`Z0`,
`C1`/`C0`, `SU`/`SD`/`SL`/`SR` when the stick leaves its dead zone (and
again every 350 ms while it stays there), `S0` when it returns, plus a
heartbeat of both buttons once a second. The Mac side reads
`/dev/cu.usbmodem*` with the `serialport` crate, `default-features =
false` so Linux CI needs no libudev. The same port carries lines down:
`P` is answered with `P`; `W<n>` sets how many amber pixels the LED
strip shows while no button is held.

Verified on 2026-09-25 with this crate against the Feather on
`/dev/cu.usbmodem11301`:

```
cargo run --release
```

```
[   0.009] opened /dev/cu.usbmodem11301
[   0.016] pong 6.5 ms
[   1.004] C0
[   1.005] Z0
[   1.005] S0
[   1.208] pong 1.8 ms
...
[   7.629] pong 4.3 ms
```

- Round trip (a line down, its answer back) 2 to 9 ms over eight
  pings. A press will land in the next frame.
- The heartbeat arrives every second, so a connection made mid-hold
  syncs within a second.
- The idle stick reads `S0` throughout: a dead zone of 60 raw units
  holds at rest.
- Opening the port is 9 ms; `available_ports` lists each device as
  both `tty.` and `cu.`, so the reader maps to `cu.` and dedups.

Recommendation: a `Controller` port polled every frame, fed by a
thread that owns the serial port and reconnects after a second when a
read fails. Events enter the core as `AppAction::Controller`; the
core keeps the selection and the held session; the UI syncs the voice
runtime to the held session through the microphone's own path.
