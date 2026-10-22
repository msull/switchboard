# Response to the plan feedback for #230, round 1

All edits are in `review/1/repo/plan.md`. The earlier plan at
`plan/1/repo/plan.md` is untouched.

The premises the review settled (fragment not sent, `Origin` on the
handshake, preflight on a custom header, no `Origin` on a same-origin
GET, one invite parser, pin on the key) needed no change. The plan's
"not in the facts file" notes on them stay as written; the review is
now their source.

## 1. Reissue at every bind: same issuer and serial, new leaf per launch

Changed. Sections: the header, Where this departs (the certificate
bullet), The record, The signals, The actions and effects, The tests,
Docs, Contract additions.

- **Random serial.** Each issue sets `serial_number` to 16 random bytes
  from `fill_random`, top bit cleared (`rcgen-0.14.10/src/lib.rs:878`).
  An adapter test checks that two issues over one key have different
  serials, and that both differ from the serial of `make_identity`'s
  certificate.
- **Stable leaf.** The served leaf is persisted as an additive
  `#[serde(default)] served: Option<ServedCertificate>` on `Identity`,
  under `IDENTITY_SCHEMA_VERSION` 1 unchanged, the way `listen` was
  (`src/ports/remote.rs:39-40`). There is no new file. The field goes
  through the existing `SaveIdentity` path, so no port method is added.
- **When it is reissued.** Only when `served_is_current` (a pure core
  function, with a test) is false: the leaf is missing, it names
  another address, or it expires within 30 days. It is stored through a
  new `AppAction::ServedCertificateMade`.
- **Older builds.** An older build ignores the field, and a save by it
  drops the field. That costs one more warning, and the plan says so.
- **Spike check.** Spike 23 gains "relaunch Switchboard, reload: does
  Safari warn again?"
- **New test** in `tests/remote.rs`: the served DER is the same after
  the app is rebuilt over the same store.

## 2. Decision 1 drops two §5 guarantees on an unanswered owner question

Changed. Sections: Decisions 1 and 6, The actions and effects
(`remote_pair`), The UI, The tests, Docs §5, Contract additions,
CHANGELOG.

- **The weakening, stated as the owner's decision** (facts:56-58).
  Decision 1 now quotes `docs/remote-client.md:309` and `:312` and says
  what a browser loses against each.
- **The build: a browser pairs `read` whatever the window's scope.** The
  host knows a browser by `"cookie": true` on `/pair`. The owner raises
  the device by hand in the devices dialog, which §5 allows at `:314`.
  So a relayed code yields a reader, never a device that types into
  agent sessions.
- **UI.** The dialog and the scope combo's hover say what raising a
  browser trusts. The page says how to raise it and rereads `/device`,
  so a raise shows without re-pairing.
- **Tests.** A control window pairs a browser as `read`, `send` is 403
  `scope` until `SetDeviceScope`, and a bearer pairing still gets
  `control`.
- **Fallback if the owner rejects the click-through:** the facts:58
  download route. That means `GET /certificate` before pairing, CA:TRUE
  plus SAN, EKU and ≤825 days, the Certificate Trust Settings steps in
  the dialog, and browsers keeping the window's scope.
- **Spec rewrite.** §5's lines 309, 312 and 317 are rewritten with the
  native and browser rules side by side, not added to (Docs).
- **Acceptance.** It now includes the raise step.

## 3. The SAN and the QR's address can disagree

Changed. Sections: Decision 3, The record (`ServedCertificate::address`),
The actions and effects, The UI, The tests.

- **One source for the QR.** `invite_url` now takes its host from
  `identity.served.address`, the address the served leaf names, so the
  QR and the SAN have one source. `local_address()` leaves the dialog.
- **Rebind on a network change.** After `OpenPairing`, the app compares
  `local_address()` with that address. When they differ it rebinds,
  which reissues the leaf.
- **Test seam.** The lookup goes through an `address_source` seam,
  beside `set_remote_port_override` (`src/app.rs:2920`).
- **Tests.** The core test checks that `invite_url`'s host is the
  leaf's address. The new `tests/remote.rs` test changes the address
  source, opens pairing, and checks that the served leaf and the
  invite both name the new address.

## 4. The device name cannot say "Safari on iPad"

Changed. Section: Decision 6.

A Mac user agent with `navigator.maxTouchPoints > 1` is named "Safari
on iPad". The decision cites the forum thread and says the heuristic
misnames a touch-screen Mac.

## 5. The origin rule refuses a non-default fetch mode

Changed. Sections: Decision 2, The actions and effects (`authenticate`),
The tests.

- **Default mode only.** The page uses `fetch`'s default mode (`cors`)
  and never sets `mode` or `referrerPolicy`. The plan gives the reason
  (Fetch step 4.1 under `no-referrer`).
- **Both sides of the rule stated.** `authenticate` says a missing
  `Origin` is accepted and `Origin: null` is refused.
- **Test.** The cookie test checks that `Origin: null` on `/device` and
  on `send` returns 403 `cross_origin`.

## 6. A paired browser's static loads can fill the stranger pool

Changed. Sections: Decision 4, The signals (`connection`), The actions
and effects (`serve_remote`), The tests, Spike 23.

- **Took the review's first option.** A static request carrying a
  `switchboard-<port>` cookie goes to the frame. The frame serves the
  same bytes and answers `authenticated: true` when `device_by_token`
  knows the cookie, so the connection moves to the paired pool
  (`src/adapters/remote.rs:835-848`). A static request without a cookie
  is still answered on the connection thread.
- **Test.** `MAX_UNAUTHENTICATED` cookie-bearing `/app.css` connections
  stay open and an unpaired `/info` is still answered; without the
  cookie, that connection is closed at accept.
- **Unpaired loads.** The 30 s cost of an unpaired load is in the
  Spike 23 checklist, tied to the 4 → 8 fallback.
- **Line numbers.** I cited the idle timeout as `:573` and the
  point where it is set as `:785`. The review's `:574` and `:782` are
  off by a few lines in this tree.

## 7. Line references

Changed. Sections: The signals, Decision 5, The tests.

- **`listener.local_addr()`:** `:678` is now `:673`.
- **The stalled-strangers test:** `:3225` is now `:3226`.
- **The embedding precedent:** decision 5 now says the precedent is
  `include_bytes!` at `src/main.rs:167`, and that the tree has no
  `include_str!` yet.
