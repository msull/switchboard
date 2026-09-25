"""Report nunchuk buttons and stick over USB serial for Switchboard.

Lines up (device to Mac), each newline-terminated:

    Z1 / Z0   Z pressed / released
    C1 / C0   C pressed / released
    SU SD SL SR   the stick left the dead zone in that direction
              (repeated every REPEAT_S while it stays there)
    S0        the stick came back to centre
    P         reply to a ping
    # ...     a comment, ignored

Button and stick states repeat once a second as a heartbeat so the Mac
side syncs up if it connects mid-hold.

Lines down (Mac to device):

    P         ping; answered with P
    W<n>      n agents waiting: n amber pixels while no button is held

The LED strip animates while a button is held (rainbow for Z, cyan for
C) and shows the waiting count otherwise.
"""

import sys
import time

import board
import supervisor

import adafruit_nunchuk
import neopixel
from adafruit_led_animation.animation.comet import Comet
from adafruit_led_animation.animation.rainbowcomet import RainbowComet
from adafruit_led_animation.color import AMBER, CYAN

HEARTBEAT_S = 1.0
REPEAT_S = 0.35       # stick held against an edge repeats this often
REPEAT_DELAY_S = 0.5  # ...after this first pause
DEAD_ZONE = 60        # stick units from centre (raw 0..255, centre ~128)
NUM_PIXELS = 144

pixels = neopixel.NeoPixel(board.A0, NUM_PIXELS, brightness=0.1, auto_write=False)
pixels.fill(0)
pixels.show()
# animate() is non-blocking: it only draws when a frame is due, so the loop keeps polling
animations = {
    "Z": RainbowComet(pixels, speed=0.01, tail_length=24, bounce=True),
    "C": Comet(pixels, speed=0.01, color=CYAN, tail_length=24, bounce=True),
}

nc = None
held = {"Z": None, "C": None}
active = None  # button whose animation is showing: the first one pressed
last_sent = 0.0
stick = "0"          # "0" or one of U D L R
stick_since = 0.0
stick_repeat_at = 0.0
waiting = 0
idle_dirty = True
inbound = ""


def show_idle():
    pixels.fill(0)
    for i in range(min(waiting, NUM_PIXELS)):
        pixels[i] = AMBER
    pixels.show()


def stick_direction(x, y):
    dx, dy = x - 128, y - 128
    if abs(dx) < DEAD_ZONE and abs(dy) < DEAD_ZONE:
        return "0"
    if abs(dx) >= abs(dy):
        return "R" if dx > 0 else "L"
    return "U" if dy > 0 else "D"


def update(new_held, direction):
    global active, last_sent, stick, stick_since, stick_repeat_at, idle_dirty
    now = time.monotonic()
    heartbeat = now - last_sent >= HEARTBEAT_S
    for name, down in new_held.items():
        if down != held[name] or heartbeat:
            print(f"{name}{1 if down else 0}")
        held[name] = down

    if direction != stick:
        stick = direction
        stick_since = now
        stick_repeat_at = now + REPEAT_DELAY_S
        print(f"S{stick}")
    elif stick != "0" and now >= stick_repeat_at:
        stick_repeat_at = now + REPEAT_S
        print(f"S{stick}")
    elif heartbeat and stick == "0":
        print("S0")
    if heartbeat:
        last_sent = now

    previous = active
    if active is not None and not held[active]:
        active = None
    if active is None:
        for name in ("Z", "C"):  # CircuitPython's next() takes no default
            if held[name]:
                active = name
                break
        if active is not None:
            animations[active].reset()
        elif previous is not None or idle_dirty:
            show_idle()
            idle_dirty = False


def read_inbound():
    """Handle complete lines from the Mac without ever blocking."""
    global inbound, waiting, idle_dirty
    while supervisor.runtime.serial_bytes_available:
        inbound += sys.stdin.read(1)
        if not inbound.endswith("\n"):
            continue
        line, inbound = inbound.strip(), ""
        if line == "P":
            print("P")
        elif line.startswith("W") and line[1:].isdigit():
            waiting = int(line[1:])
            idle_dirty = True


while True:
    read_inbound()
    try:
        if nc is None:
            nc = adafruit_nunchuk.Nunchuk(board.I2C())
        buttons = nc.buttons
        x, y = nc.joystick
        update({"Z": buttons.Z, "C": buttons.C}, stick_direction(x, y))
    except (OSError, RuntimeError, ValueError) as e:
        # nunchuk unplugged or I2C glitch: report released, retry shortly
        print(f"# nunchuk error: {e}")
        nc = None
        update({"Z": False, "C": False}, "0")
        time.sleep(0.5)
        continue

    if active is not None:
        animations[active].animate()
    time.sleep(0.002)
