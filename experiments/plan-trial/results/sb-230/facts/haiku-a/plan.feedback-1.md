# Feedback 1 on plan.md (issue #230): premises

Checked against the worktree at `7fe8949` and against `facts/1/repo/facts.md`. Objections are ordered most severe first. Each names the decision, the premise it rests on, what supports the premise (a facts line, a tree line, or nothing), and the fix.

1. **A default session cookie makes the page say "revoked" after every browser restart.**
   - Decision: "No code and no token" (UI). A stored pinned key that matches `/info` gives "This browser was revoked on the Mac." Trust decision 2 builds a session cookie, which is lost on close (the plan's own note; facts, For the owner, token transport: "Session means re-pair after each browser close").
   - Premise: a 401 with a matching stored key means the device was revoked. Support: nothing. The tree answers a missing credential and an unknown token differently: no cookie or bearer gives "pair this device first" (`src/app.rs:3128-3135`); an unknown token gives "the token is not known; pair again" (`src/app.rs:3141-3148`). A browser whose session cookie has ended sends no credential, so the rule above labels a normal close as revocation.
   - Fix: say "revoked" only when the reply's message is "the token is not known", or use one sentence that covers both ("This browser's access on the Mac has ended. Scan a new code to pair again."). Write the choice into the UI section.

2. **"Revoked and never-paired look the same on the wire" is wrong for never-paired.**
   - Decision: Where-departs bullet 9 says all three states are "the 401 at `src/app.rs:3142-3148`". Facts, Tree, says the same. The page's three-state logic depends on this.
   - Premise: the states are indistinguishable on the wire. Support: the tree disagrees. A never-paired browser sends no bearer and gets "pair this device first" (`src/app.rs:3128-3135`). A revoked browser and an identity-changed browser (key rotation clears `devices`, `src/app.rs:2816`) both send an unknown token and get "the token is not known" (`src/app.rs:3141-3148`). The status and the code (`unauthorized`) are the same for all three.
   - Fix: correct bullet 9 and the facts line. Then say whether the page reads `error.message` (brittle) or the plan adds a distinct code. A new code is an additive wire change and belongs in Contract additions.

3. **The four-slot unauthenticated pool: the decision rests on unresolved lines with no fallback written.**
   - Decision: Actions, static routes: "the four-slot pool bounds concurrency." Docs §4.1 keeps the four as "the pairing bound."
   - Premise: how many sockets a browser opens for one page load, and whether an idle keep-alive socket holds a slot. Support: facts, Unresolved, lists both as "Needs a run."
   - Tree: a socket is counted at accept and released only when its thread returns (`src/adapters/remote.rs:701-720`). `/info` and `/pair` replies go out with `Connection: keep-alive` (`src/adapters/remote.rs:545`), so each idle socket holds a slot for `IDLE_TIMEOUT`, 30 s (`:573`). Past the pool, accept drops the socket without an answer (`:701-707`). A page load (`/`, `/app.js`, `/app.css`, then `/api/v1/info`) can use up to four slots, and `/pair` then gets no answer.
   - Owner item "Static pool" says "The plan chooses", but the body chooses nothing.
   - Fix: choose now. For example, close `/info` and `/pair` after their reply so slots free at once, or give static answers their own count. State the bound, and add a test that opens the page's sockets and then pairs.

4. **The reply writer, as written, cannot write a static response.**
   - Decision: Actions, "Reply writer": `respond()` writes the new headers "after the fixed head."
   - Premise: the fixed head can stay. Support: the tree says no. The fixed head is `Content-Type: application/json` and `Connection: keep-alive` (`src/adapters/remote.rs:545`). Facts, Tree, says a static response needs a header-capable writer.
   - Result: a static answer carries two `Content-Type` and two `Connection` headers, and keep-alive where the plan requires `Connection: close`.
   - Fix: say the fixed head is built from the headers when given (the defaults drop out), or that static answers use a separate writer. Name the function.

5. **Trust decision 2's fallback cannot be built from the plan as written.**
   - Decision: fallback = token in page memory, sent as a bearer on `fetch` and as a `Sec-WebSocket-Protocol` value.
   - Premise: the cookie is kept under the click-through. Support: facts, Unresolved.
   - The fallback does not hold together. The browser pairing reply has no `token` (Actions, Pairing: "replies with {scope, server_id, device_id} and no token"), so page memory never receives one. `read_request` keeps only `host`, `origin` and `cookie` (Actions), not `Sec-WebSocket-Protocol` (`src/adapters/remote.rs:444-514`). Contract additions list neither change.
   - The gate says the fallback "replaces the cookie branch before merge", but nothing is written to replace it with.
   - Fix: write the fallback in full: the reply that returns the token in that mode, the subprotocol header parse, and the contract line. Or say that a failed cookie check blocks the merge, and drop the word "fallback."

6. **Trust decision 1 has no fallback if the certificate warning is a hard failure.**
   - Decision: "The click-through is the trust step."
   - Premise: a browser shows a warning the owner can proceed past. Support: facts, Unresolved, says "an interstitial the owner can proceed past, or a hard failure. Needs a device." The certificate has no SAN for an IP host (`src/adapters/remote.rs:102-107`).
   - Only the per-session wording is written as a fallback. If a browser hard-fails, the trust step does not exist, and the only other route named (the iOS trust profile) is one the plan rejects.
   - Fix: write the hard-fail branch: what the pairing window says and which route the owner takes. Or make the gate fail the merge on a hard fail. Also say why no IP SAN is added (the address varies by network).

7. **The spike rule is contradicted, and the cookie is built before its mechanism is verified.**
   - Rule: CLAUDE.md, Adding a feature and the spike rule: "add a new spike directory for any new risky mechanism before building on it."
   - Decision: Where-departs 12 adds no spike. Steps (3) builds the cookie branch, the Origin check and `/api/v1/device`. The phone run is the gate "before the commit merges," after the build.
   - The plan supports the decision with facts' "Spike decision: No spike," which is not the tree. The rule is CLAUDE.md, and the plan does not cite it against the decision.
   - Fix: add `spikes/<n>-browser-session/README.md` with the owner's run as its recorded result, and place it before Step (3). If the owner wants no spike, the plan must say that CLAUDE.md is being overridden, and who decided.

8. **"No schema bump" cites facts for a claim facts does not make.**
   - Decision: Record, Identity and Devices: no `IDENTITY_SCHEMA_VERSION` bump, because "ports/remote.rs types are outside" CLAUDE.md's rule. Cited as "(facts, Tree)."
   - Facts, Tree, has no such line.
   - Support: CLAUDE.md's text, which covers "any type in `model.rs` that is serialized." The tree has `IDENTITY_SCHEMA_VERSION` in `src/ports/remote.rs:17`, written at `src/adapters/remote.rs:118`. The loader deserializes with no version check (`src/adapters/remote.rs:55-64`).
   - Fix: cite CLAUDE.md. Say why `1` is kept: the new field is `#[serde(default)]`, and no reader refuses an unknown version.

9. **The commit message omits the contract additions CLAUDE.md requires.**
   - Rule: CLAUDE.md, Extending the contract: extend `src/ports/` additively and "say what you added in the commit message."
   - Plan line 169 gives a one-line title only. The additions are `PairedDevice.browser`, `Identity.certificate_expires`, and the new parameter of `Core::new_device`.
   - Fix: the commit body lists these.

10. **Struct literals are missed, and two counts are wrong.**
    - Adding `browser` to `PairedDevice` breaks every struct literal: `src/core/tests.rs:11781`, `src/adapters/remote.rs:991`, `tests/remote.rs:1430`, `tests/ui.rs:7427`, plus `src/core/remote.rs:167` (`new_device`).
    - Adding `certificate_expires` to `Identity` breaks: `src/core/tests.rs:11769`, `src/adapters/remote.rs:117`, `tests/ui.rs:7436`.
    - Steps (1) names none of them, so the build fails at Step (1).
    - `RemoteReply {` has 29 construction sites (28 in `src/app.rs`, one at `src/adapters/remote.rs:829`). The plan says 30. The thirtieth match is the definition at `:389`.
    - `RemoteRequest` has one construction site (`src/adapters/remote.rs:506`). The plan says three.
    - Fix: list the sites, and correct both counts.

11. **The reissue ignores the read-only gate.**
    - Decision: Startup certificate: the reissue is "saved through `save_identity`" and dispatched. The plan does not mention read-only.
    - Premise: the reissue may write. Support: the tree gates every write in `load_identity`: `src/app.rs:2807` (`Ok(_) if self.core.read_only() => (identity, false)`) and `:2833` (`None if self.core.read_only() => return`). Spec §4.1 says "a read-only instance refuses to bind" (`docs/remote-client.md:134`).
    - Fix: the reissue runs only when the instance is not read-only. Add a test.

12. **The spec amendments miss two spec lines the plan contradicts.**
    - Decision: the card excerpt is not on the wire (Where-departs 1).
    - `docs/remote-client.md:207` says the excerpt is `last_message` on the record and "moves with `changed {session}`." Line 201 says the card excerpt is "masked from the turn's full text, then cut."
    - Tree support for the plan: no excerpt field on `SessionView` (`control/src/lib.rs:646-662`); no `last_message` on the model (`src/core/model.rs`); the card computes it from the transcript (`src/ui/cards.rs:558`). The plan cites this, which is fine, but the Docs section does not amend lines 201 or 207.
    - Fix: add §4.2 amendments for lines 201 and 207.

13. **"The page can do without a Rust projection" is uncited, and the departure is misdescribed.**
    - Decision: Client-side logic: "The page can do without it."
    - Premise: the page's reconcile needs no core projection. Support: nothing in facts. The tree partly supports it: every envelope carries `revision` (`src/app.rs:3717-3741`), and spec §4.4 gives subscribe-then-snapshot (`docs/remote-client.md:260-267`). Cite those.
    - Also, the plan says it "departs from CLAUDE.md's 'logic moves to the core'." CLAUDE.md states that rule for `src/ui/`. The page is outside `src/ui/`. The accurate statement is that a new client-side logic layer is outside CLAUDE.md's architecture.

14. **The `history.replaceState` decision rests on an unresolved line with no fallback.**
    - Decision: the page calls `history.replaceState(null, '', '/')` before any request, to keep the code out of history.
    - Premise: the visited-URL record drops the code. Support: facts, Unresolved, says it is not known.
    - The tree gives a fallback the plan does not write: the code is single-use ("A consumed code is refused on the next `/pair`," `docs/remote-client.md:479`), and the window closes after five minutes (`src/core/action.rs:972`).
    - Fix: write that as the fallback, and state the residual exposure.

15. **The Origin gate rests on an unresolved form with no fallback.**
    - Decision: cookie-authenticated state changes and the events upgrade need `Origin` equal to `https://` plus Host (Actions, Authentication).
    - Premise: browsers send `Origin` as `https://<ip>:<port>`. Support: facts, Unresolved, says it is assumed, not checked.
    - If the browser's form differs, every page send and the events socket get 403 `origin`, and the page stops working. The gate names a test, but no fallback.
    - Fix: compare the parsed scheme and authority with the Host header, not the whole string, or name the fallback.

16. **A session cookie creates a new device on every browser start.**
    - Decision: Trust decision 2 builds a session cookie. Pairing creates a device each time, and `new_device` mints a fresh id on each call (`src/core/remote.rs:167-174`).
    - Premise: nothing states the cost. The plan says only that "the revocation path is the device record."
    - Effect: each browser start adds a device to `devices.json` and the paired-devices dialog, with no cleanup.
    - Fix: state the cost in the plan, and either accept it or name the cleanup.

17. **The raw Cookie header would be held on `RemoteRequest`.**
    - Decision: Actions, Reply writer: `read_request` keeps `cookie`.
    - Premise: holding the raw header is safe. Support: the tree says the opposite. `RemoteRequest` derives `Debug`, and its doc says the bearer "is taken out of the headers so it is never logged with them" (`src/adapters/remote.rs:356-358`). A raw Cookie header carries the session token. CLAUDE.md's hard rules: secret values never reach a log line.
    - Fix: extract the `__Host-switchboard-<server_id>` value at read time, as the bearer is extracted, and keep only the token.

18. **Minor citation errors.**
    - (a) The headset's paste path calls `PairingInvite.parse` at `client/Sources/SwitchboardUI/BrowserView.swift:883`. `client/Sources/SwitchboardClientCore/ConnectionManager.swift:482-487` is `pair(invite:)`, which takes an already parsed invite and does not parse.
    - (b) "The host trims it to 64 characters" reads as new work. `new_device` already trims and caps at 64 (`src/core/remote.rs:169`). Say that the existing behaviour is kept.
    - (c) Where-departs bullet 9 has the wrong "All three" claim. See item 2.
    - (d) The `RemoteReply` and `RemoteRequest` counts. See item 10.
