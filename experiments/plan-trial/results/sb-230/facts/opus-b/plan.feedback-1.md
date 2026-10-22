# Feedback on the plan for #230, round 1 (premises)

Checked against the worktree at 7fe8949. `SCHEMA_VERSION` 16
(`src/core/model.rs:12`) and `IDENTITY_SCHEMA_VERSION` 1
(`src/ports/remote.rs:17`) match the plan. Every facts-file citation
in the plan points at the line it claims. The tree references are
right except the ones in objection 7. Nothing in the plan injects
keystrokes or writes into a project's `.claude/` or `.switchboard/`. No
serialized type changes, so no schema bump is needed as written (but
see objection 1). `RemoteRequest`/`RemoteReply` are adapter types, not
contract.

The premises the plan left "not in the facts file" are settled here,
with no objection:

- The fragment is not sent to the server. RFC 3986 §3.5: the fragment
  "is separated from the rest of the URI prior to a dereference" and
  "dereferenced solely by the user agent".
- A browser always sends `Origin` on a WebSocket handshake. RFC 6455
  §4.1 item 8: "The request MUST include a header field with the name
  |Origin| … if the request is coming from a browser client." The Fetch
  standard's "append a request `Origin` header" step 3 appends it for
  mode `websocket` whatever the method.
- A custom request header forces a preflight. Fetch, HTTP fetch step
  4.1: a CORS preflight runs when "there is at least one item in the
  CORS-unsafe request-header names with request's header list".
  `X-Switchboard-Page` is not safelisted. `serve_remote` answers an
  `OPTIONS` with 404 off `/api/v1/` (`src/app.rs:3023-3025`) or 401
  from `authenticate` on it, never a 2xx with
  `Access-Control-Allow-*`, so the cross-origin request is never
  sent.
- A same-origin `GET` carries no `Origin`. Fetch, same algorithm: with
  no `cors` tainting and no websocket mode, a GET or HEAD gets none. So
  "origin absent" has to be accepted, as the plan does.
- The JSON invite has one parser. All three callers (`main.swift:111`,
  `BrowserView.swift:106`, `:883`) go through `PairingInvite.parse`
  (`client/Sources/SwitchboardProtocol/Info.swift:114`). `Address.init`
  already strips IPv6 brackets
  (`client/Sources/SwitchboardTransport/HostClient.swift:15-25`).
- The native client pins the key, not the certificate bytes
  (`client/Sources/SwitchboardTransport/Pinning.swift:18-32`), so a
  reissued leaf does not break it.

## Objections

1. **Reissuing the leaf at every bind gives every certificate the same
   issuer and serial, and a different certificate each launch.**
   Premise: "the host now reissues the leaf at every bind … the served
   leaf is derived from it at bind and never saved" (plan lines 37-38,
   173-174, 197-211). It rests on facts:12 and facts:60, which settle
   only that *pins* survive. Nothing supports the claim that a
   *browser's* trust survives. The tree contradicts it in two ways:

   - **The serial never changes.** rcgen sets the serial to the first
     20 bytes of SHA-256 of the public key when `serial_number` is
     `None` (`rcgen-0.14.10/src/certificate.rs:458-468`), and the plan
     keeps the CN `Switchboard <server_id>`
     (`src/adapters/remote.rs:104-107`). Every reissue, and the
     original certificate in `identity.json`, therefore has the same
     issuer and serial with different contents. That breaks RFC 5280
     §4.1.2.2 ("the issuer name and serial number MUST identify a
     unique certificate"), and Firefox refuses such a certificate as
     `SEC_ERROR_REUSED_ISSUER_AND_SERIAL`.
   - **The leaf changes on every launch.** `not_before = now − 1 day`
     makes a new leaf at each bind, and an ECDSA signature differs
     each time anyway. A browser's click-through is for the certificate
     it was shown, so the owner meets a new warning after every app
     restart.

   That trains the owner to click through warnings on this origin,
   which is the habit an intermediary needs (objection 2). Fix:
   - Set an explicit random serial on each issue.
   - Keep the served leaf stable: persist it and reissue only when the
     names change or expiry is near. Persisting it in `identity.json`
     is a serialized change: say whether it is an additive
     `#[serde(default)]` field under `IDENTITY_SCHEMA_VERSION` 1, or a
     bump with a migration. A separate file in the data directory also
     works; say which.
   - Add the spike check: does a relaunch of the app re-warn?

2. **Decision 1 drops two guarantees of `docs/remote-client.md` §5 on
   an owner question that has no answer, and the fallback it writes
   covers a different question.** Premise: "the click-through is the
   trust step." Facts list this as an owner decision (facts:56-58), not
   a settled line, and the plan's only written fallback (lines 76-81)
   is for whether `wss://` opens (facts:53). Nothing covers the trust
   choice itself. The settled spec says the opposite, in two places:

   - "Pairing verifies the key before the code is sent. The code alone
     would let an intermediary … forward the code to the real host, and
     take the token" (`docs/remote-client.md:309`).
   - "after pairing, every connection verifies the pin before the token
     is presented" (same line).

   With the page, an impostor's page reads `location.hash` and relays
   the code with the real key, as the plan admits (line 72). After
   pairing, the cookie goes to whatever certificate the owner next
   clicks through, which objection 1 makes routine. This window can
   grant `control`, which types into agent sessions. The plan should
   state the weakening as a decision for the owner, not a dialog
   sentence, and write a fallback beside it. Options:
   - A browser pairing is capped at `read` whatever the window's scope,
     and raised by hand in the devices dialog (§5 already allows
     that).
   - The facts:58 download route.

   Whichever it is, `docs/remote-client.md:309` and `:312` must be
   rewritten, not just added to.

3. **The certificate's SAN and the QR's address can disagree.** Premise:
   "`bind_remote` passes `local_address()` into the names" (plan line
   209), while the QR takes `local_address()` "once per code" when the
   pairing window opens (plan line 246). `bind_remote` runs only at
   listen changes (`src/app.rs:2936-2985`), and nothing rebinds when
   the Mac's address changes. So a pairing opened after a network
   change shows a URL whose host the served leaf does not name. Whether
   iOS offers a click-through at all for a name mismatch is Unresolved
   (facts:44). Fix: take the QR's address from the names the bound
   leaf carries, or reissue and swap the TLS config when the pairing
   window opens on a different address. Then add a core or adapter test
   that the two agree.

4. **The device name cannot say "Safari on iPad".** Premise: decision 6
   tells iPad from Mac by `navigator.userAgent` (plan lines 141-143).
   Nothing in facts or the tree supports it. Since iPadOS 13, Safari on
   iPad sends the Mac user agent by default ("Macintosh; Intel Mac OS
   X"), per the Apple developer forum thread "User Agent in Safari on
   iPadOS" (https://developer.apple.com/forums/thread/119186). Either
   add the touch heuristic (`navigator.maxTouchPoints > 1` with a Mac
   user agent means an iPad), or drop "Safari on iPad" from the list and
   say an iPad shows as "Safari on Mac".

5. **The origin rule refuses a page whose fetch mode is not the
   default.** Premise: a cookie request passes when `Origin` is absent
   or equals `https://{host}` (plan lines 223-226). The plan also sends
   `Referrer-Policy: no-referrer` on the page (line 116). By Fetch's
   "append a request `Origin` header" step 4.1, a non-GET request whose
   mode is not `cors` and whose policy is `no-referrer` sends
   `Origin: null`. A `fetch(.., {mode: "same-origin"})` POST to `/pair`
   or `send` would then be refused as `cross_origin`. Say that the page
   uses `fetch`'s default mode (`cors`), and add a test that
   `Origin: null` is refused, so the rule is pinned from both sides.

6. **A paired browser's static loads can fill the stranger pool.**
   Premise: "a connection moves to the paired pool on its first cookie-
   or bearer-authenticated API request" (plan lines 125-126). The
   fallback for parallel connections (4 → 8) is written. What is not
   written: a connection that fetched only `/app.css` stays in the
   unauthenticated pool for the 30 s idle timeout
   (`src/adapters/remote.rs:573`, `:785`), and the accept loop drops
   any new connection past 4 (`:701-705`). A reload by a paired phone
   can therefore lock out its own `/device` request, and anyone
   else's `/info`, for 30 s. Either let a request carrying a known
   cookie move the connection to the paired pool even on a static path
   (the static answer would then need the frame, or a token check on
   the thread), or put the 30 s cost in the spike's checklist with the
   4 → 8 fallback tied to it.

7. **Line references to correct.**
   - `listener.local_addr()` in `bind` is `src/adapters/remote.rs:673`,
     not `:678`.
   - `stalled_strangers_fill_their_own_pool_and_never_a_paired_clients`
     is `tests/remote.rs:3226`; `:3225` is its `#[test]`.
   - `src/main.rs:167` is an `include_bytes!`, not `include_str!`. There
     is no `include_str!` precedent in the tree. Say "the precedent for
     embedding is `include_bytes!` at `src/main.rs:167`", or cite
     nothing.
