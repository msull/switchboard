# Review round 1: premises

Every citation below was checked against the worktree at 7fe8949
(0.4.1). The plan's line references to `src/app.rs`, `src/adapters/remote.rs`,
`src/core/remote.rs`, `src/core/model.rs`, `src/ports/remote.rs`,
`src/ui/remote.rs`, `src/ui/cards.rs`, `src/script.rs`, `src/main.rs`,
`src/ui/theme.rs`, `control/src/lib.rs`, `tests/remote.rs` (245, 299, 406,
682, 1406, 1587, 1829, 1960, 3226), `tests/ui.rs`, `Info.swift`,
`main.swift`, `BrowserView.swift`, `FixturesTests.swift`, README rows and
`docs/design.md` 3107-3584 all match. The schema versions (16, 7, 1) match
and no serialized type changes, so no bump is right. The cookie as the
token's home, with the bypassed-certificate test named as the
implementer's first step, is the right call and is not objected to. The
fragment over the query rests on `read_request` splitting the target at
`?` (`src/adapters/remote.rs:494`) and `refuse` logging `request.path`
(`src/app.rs:3763-3766`): settled by the tree. Four objections.

## 1. `PairingInvite.parse` cannot reach `Address.init?(_:)` (Decision 3)

Premise: the URL's authority is parsed "through `Address.init?(_:)`,
`HostClient.swift:15-25`, which already brackets IPv6". Against the tree
on two counts.

- `Address` lives in `client/Sources/SwitchboardTransport/HostClient.swift:5`,
  and `PairingInvite` in `SwitchboardProtocol`. `client/Package.swift:18-19`
  makes `SwitchboardTransport` depend on `SwitchboardProtocol`, not the
  reverse, so `parse` cannot call `Address` without a dependency cycle.
- `Address.init?` strips brackets (`HostClient.swift:20-22`); it does not
  add them. `URLComponents.host` for an IPv6 literal is the bare address,
  so a parse built on it would hand `addresses` an unbracketed host that
  the later `Address.init?` then splits at the wrong colon.

Fix that keeps the decision: `parse` takes the authority as text, the
substring between `https://` and the first `/`, straight into
`addresses: [that]`, which keeps the brackets the host wrote and needs
nothing from Transport. The three-argument CLI path at
`main.swift:112-118` is the one that goes through `Address` and is
unchanged.

## 2. The stranger-pool count omits the socket and the kept connections (Decision 4)

Premise: "a load is three files on at most three parallel connections,
one below `MAX_UNAUTHENTICATED`, leaving one for a concurrent `/info`".
The tree says otherwise. A connection leaves the unauthenticated count
only on its first authenticated reply (`remote.rs:831-845`) or when its
thread ends; every reply is `Connection: keep-alive` (`remote.rs:548`),
and after the first request a stranger's read timeout is `IDLE_TIMEOUT`,
30 s (`remote.rs:785`, `remote.rs:573`), so the three page-load
connections stay counted for up to 30 s after the HTML, CSS and JS are
served. Step 5 opens `wss://…/api/v1/events` "before the snapshots", on a
fourth connection, so at that moment the pool holds four: the slot "for a
concurrent `/info`" does not exist, and a browser that opened a
speculative second connection during the load (Chrome does) has the
socket itself dropped at `remote.rs:699-705`. The page still works
because the poll fallback catches a failed socket, but the plan's count
is the design and it is wrong as stated.

Robust default with no browser fact needed: make the first authenticated
request a `fetch` (`GET /api/v1/waiting`), which moves its connection to
the paired pool and frees a stranger slot, and open the socket after
that reply, buffering and applying by revision as §4.4 says (subscribe
before the *collections* snapshot, not before every request). State the
count as: three load connections, one moved by `/waiting`, then the
socket as the fourth stranger. The test
`a_page_load_fits_the_stranger_pool` should hold three page connections
open and then open the socket, not just three parallel `GET`s.

## 3. The excerpt departure rests on a premise the spec contradicts (Where this departs)

Premise: adding the excerpt "would put transcript text on a metadata
reply, outside the content worker's redaction path (§4.1 … §4.3
redaction)", and the spec sentence at `docs/remote-client.md:207` "names a
field the tree does not have". Two corrections:

- `docs/remote-client.md:201` already lists "**last response** and the
  card excerpt: masked from the turn's full text, then cut" among the
  redacted reads. The spec placed the excerpt on the redaction path; the
  plan's reason for leaving it out is not that it cannot be redacted but
  that no wire view carries it yet, which is true (`SessionView`,
  `control/src/lib.rs:637-688`).
- `last_message` does exist in the tree: on the hook event
  (`src/ports/events.rs:24`, filled by `src/adapters/hooks.rs:196` from
  the hook's `last_assistant_message`, `src/bin/switchboard-hook.rs:37`).
  The core discards it; the record and the wire view have no field. So
  "on the record" at 207 is inaccurate, not the field's existence.

Leaving the excerpt out of this cut is a scope departure the plan flags,
and is acceptable. The objection is to the spec edit: rewriting 207 to
"the list has no excerpt" makes 201 and 207 contradict each other and
turns a known gap into a design statement the issue did not ask for.
Write 207 as "the excerpt is not yet on the wire; it will be a redacted
field of the session view as 201 says" and list it under known gaps in
design.md, as the plan's follow-up already does.

## 4. Line references that do not match the tree

- "`docs/remote-client.md:310` on single use" (twice, in step 1 and in
  the fragment note): the single-use sentence is line 311 ("**The code**
  …"); 310 is the settings-menu sentence. And "`docs/remote-client.md:305-308`
  (one self-signed certificate, SPKI pinned)" is 307-308; 305 is blank
  and 306 is the "Require pairing" line the plan amends separately.
- "the `/other` line in `info_is_served_to_anyone_and_every_other_read_is_refused_unpaired`
  (467)": the `/other` call is `tests/remote.rs:464`; 467 is inside the
  following `POST /api/v1/pair` call. The test begins at 426.
- "`HostClient.swift:15-25`" is under `client/Sources/SwitchboardTransport/`,
  not beside `Info.swift` as the sentence reads (see objection 1).
