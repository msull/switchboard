# Response to plan feedback 1

## 1. The reissue ran only at bind, while the QR reads the address at draw time

Agreed. The plan changed in five places:

- **The record, "Where it runs"** (rewritten). A single
  `SwitchboardApp::refresh_certificate` does the check and the swap. It
  is called from four places:
  - `bind_remote`;
  - right after the app dispatches `OpenPairing`, before the next frame
    draws the QR;
  - on every once-a-second poll while the pairing window is open;
  - on `Tick` at most once an hour otherwise, which covers the 30-day
    renewal in a long-running app (Facts, Unresolved, line 27).

  It records the address it checked in `certified_address`. The address
  comes from `self.local_address()`, which tests can override with
  `set_local_address_override`.
- **Departures, the certificate bullet.** Says when the check runs, and
  that the QR prints the address the served certificate carries.
- **Actions and effects, Adapter.** New bullet "The certificate is
  swappable without a rebind":
  - `tls_config` uses `with_cert_resolver` over a `CertSlot`;
  - `RemotePort::set_certificate` replaces the slot's value.

  I chose this over the rebind the feedback proposed. A rebind drops the
  old `Shared`, so a paired connection's next request would get 503 from
  a dead channel. With the swap, open connections and event sockets stay
  up, and only new handshakes get the new certificate.
- **Actions and effects, App, and The UI, the Mac.**
  - `DrawCtx` gains `remote_address`, filled from `certified_address`.
  - The QR and "Copy invite" build the URL from it instead of from
    `remote::local_address()` (`src/ui/remote.rs:156-158`).
- **The tests.**
  - New adapter unit for `set_certificate`.
  - New integration test
    `opening_pairing_after_the_address_moved_serves_a_certificate_for_it`.
    It binds at `10.0.0.1` and moves the override to `10.0.0.2`, then
    opens pairing. It checks that the new handshake's SAN has `10.0.0.2`
    with the same SPKI, and that the kept paired connection still
    answers 200. A move to `10.0.0.3` while the window is open is picked
    up by one poll.

## 2. The cookie is the token's only home, and its mechanics after a click-through are unresolved

Agreed. The cookie stays the store. New section **"The implementer's
first test: does the cookie hold"** comes before "Browser trust". It
says the mechanic is unsettled, cites WebKit bug 232088, and notes that
polling does not rescue a "no". Its steps:

1. Land the cookie path on the host, plus a throwaway probe page in the
   working tree. The real page replaces it before the one commit.
2. The owner runs the probe on the iPhone and on the other browsers they
   mean to support. The check is that `/pair` with `cookie: true`, then
   `GET device` with the page header, returns 200, also after a reload.
   It also checks whether `hello` arrives on the socket.
3. A yes means building the page as planned. A socket "no" alone is the
   polling default already in the plan.
4. A no on `GET device` means stop and report. The fallback is a
   script-held token sent as a bearer header and as a
   `Sec-WebSocket-Protocol` value. It is not built now; it is a new
   entry under **Follow-ups for the owner**.
