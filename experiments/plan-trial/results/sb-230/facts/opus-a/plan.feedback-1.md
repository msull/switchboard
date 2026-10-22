# Plan feedback, round 1 (premises)

I checked every line reference in the plan against the worktree at
7fe8949. They match: `src/app.rs:3023-3025, 3033, 3127-3153,
3568-3600, 3639-3647`; `src/adapters/remote.rs:101-102, 444-513, 506,
519-527, 530-552, 567, 770, 788, 797, 962, 1045`;
`src/ports/remote.rs:17, 96-107`; `src/core/model.rs:12`
(`SCHEMA_VERSION = 16`); `src/core/remote.rs:46-69, 86-92`;
`src/ui/remote.rs:160-166`; `src/ui/cards.rs:176-184, 558-574`;
`tests/ui.rs:7424, 7486-7492`; `tests/remote.rs:245, 465, 1960, 2892`;
`docs/design.md:3573`; `README.md:276-279, 383`;
`client/Sources/SwitchboardProtocol/Info.swift:113-116`;
`OP_CAP = 64` (`src/app.rs:4260`). The plan's correction of facts
`:16`/`:18`/`:19` (key and params at `:101`/`:102`, not `:105`/`:106`)
is right.

The hard rules hold. Nothing injects keystrokes. Nothing writes into a
project's `.claude/` or `.switchboard/`. No serialized type changes, so
no schema bump is needed (`devices.json` is untouched). `RemoteRequest`
gaining a field is outside the contract (facts `:15`).

The objections:

1. **The watch-set premise contradicts a settled line.** The plan says
   (line 322-323) that the whole page holds at most one watched key, so
   it "cannot crowd the headset out of the 128". The watch set is not
   only keys asked for with `watch`. It is "every content key any
   subscriber has asked to watch, **plus every content key read in the
   last minute**" (`docs/remote-client.md:254`; facts `:24` and `:69`,
   "the page fetches `last-response` per session … and spends
   watch-set slots").

   The plan's own departure, the excerpt from the first line of
   `last-response` on every waiting Claude Code row (lines 29-31,
   311-314), reads one content key per waiting session. Those reads
   happen again on every `changed session`, and on every
   `last_activity_ms` move while polling. Each read puts its key in the
   set for a minute.

   Fix: state that cost. Either bound it, for example excerpts for at
   most N waiting rows (N well under 128), fetched only on load and on
   that session's `changed`, or accept it in words, citing `:254` and
   facts `:69`. Then correct the "at most one watched key" sentence and
   the matching §5 / design-status text.

2. **A 401 on the upgrade cannot be seen by the page.** The plan
   (lines 328-331) says "a 401 on the upgrade … shows the revoked
   state". It also says the poll fallback starts when "the socket fails
   twice before `hello`" (lines 115-116). Settled against the primary
   source: the WHATWG WebSockets standard, §2.2, says a non-101
   response must "fail the WebSocket connection". §4 says user agents
   "must not convey any failure information to scripts in a way that
   would allow a script to distinguish" a server that did not complete
   the handshake from other failures. The page sees `error` and then
   `close` with code 1006, and no status
   (https://websockets.spec.whatwg.org/).

   So three cases look the same to `app.js`: a revoked or unknown token
   (a 401 from `authenticate`, `src/app.rs:3127-3153`), Safari refusing
   the click-through for `wss` (facts `:49`), and a host that is down
   or relaunching. With the plan as written:
   - A revoked device drops into "Live updates unavailable; refreshing
     every 5 s" and learns of the revocation only on its first poll.
   - A host relaunch can switch a healthy page to permanent polling.

   Fix:
   - After any socket close or failure, the page makes one
     authenticated `fetch` (`GET device`) before it reconnects or
     counts a failure. A 401 there is the revoked state. A network
     error means the host is unreachable: back off and do not count a
     failure. A 200 counts one failure toward the fallback.
   - In polling mode, retry the socket now and then (for example every
     minute), so a page that fell back after a transient failure
     returns to live.
   - Reword lines 328-331 and step 6 of the end-to-end test to match.
     Revocation closes the socket (`tests/remote.rs:2892`), and the
     next `GET device` is what says "revoked".

3. **The 404 on the connection thread is not "the same way" as the
   page files.** The plan has non-page paths answered with
   `respond(404, …)` "then a close" (lines 214-216). Its test expects
   `/favicon.ico` and `POST /` to answer "the same way" as the page
   files, a list that includes `Connection: close` (lines 347-350).
   `respond` writes `Connection: keep-alive`
   (`src/adapters/remote.rs:545`), so the reply would claim keep-alive
   and then drop the connection. That works, but the plan contradicts
   itself and the test as written would fail.

   Fix: write the 404 with a `Connection: close` head. That can be
   `respond_page`'s writer with the JSON content type, or a `close`
   flag on a private helper. Alternatively, state that the 404 keeps
   `respond`'s head, and drop it from the "same way" assertion.

4. **Moving the non-API 404 off the frame drops a refusal log the spec
   requires, and the plan does not say so.** Today a non-API path is
   refused through `refuse`, which logs
   `"remote: {method} {path} refused: {code}"` (`src/app.rs:3762-3766`).
   §5 says "Refusals are logged by op and kind, never with content"
   (`docs/remote-client.md:318`). The plan's "Nothing on the connection
   thread logs a request" (line 207) silently removes that log for
   every non-API path.

   Fix: either log the refusal on the connection thread with method and
   path only, never the query, since `/?code=` carries the code. Or
   amend `:318` and §4.1 to say that non-API paths are answered unlogged
   and why: the page's query holds a pairing code.

5. **The CSP `'self'`/`wss` premise is marked "unverified" but can be
   settled, and should be cited.** The plan (lines 230-233) leaves it as
   "Not in the facts file and unverified here", with a fallback. The
   primary source settles the spec side. CSP Level 3 §6.7.2.8 ("Does url
   match expression in origin with redirect count"), the `'self'`
   branch, matches when the hosts match, the ports match or are
   default, and "url's scheme is "https" or "wss""
   (https://www.w3.org/TR/CSP3/). §1.3 lists it as a change from Level
   2.

   On the WebKit side, Safari long refused it. WebKit bug 201591 (2019)
   was resolved as a duplicate of bug 235873, "CSP: Improve
   compatibility of source matching", which was RESOLVED FIXED
   2022-04-02 in r292266 / 249164@main
   (https://bugs.webkit.org/show_bug.cgi?id=201591,
   https://bugs.webkit.org/show_bug.cgi?id=235873). The bug does not
   name the Safari release. It holds for any iOS whose WebKit postdates
   April 2022.

   Fix: cite these in place of "unverified". Keep the per-response
   `wss://<Host>` fallback, and say it covers older WebKit. Note that a
   bare `wss:` scheme source would be the wrong fallback, because it
   allows any host.

6. **Two premises are cited as Settled where the facts file has no such
   line, and three spec lines the browser contradicts are not on the
   amend list.**
   - Line 154-156: "`localStorage` … does not need a secure context"
     is called Settled. Facts `:37` settles only that `localStorage` is
     per origin, and `:35` is about `crypto`. It is true, and you can
     cite this instead: the HTML standard's `WindowLocalStorage` IDL
     carries no `[SecureContext]`
     (https://html.spec.whatwg.org/multipage/webstorage.html). The same
     section says the getter throws `SecurityError` when the user agent
     forbids persistence. The plan's try/catch covers that, but the page
     also needs a stated behaviour for it, for example "this browser
     will not keep the pairing; pairing works for this tab only".
   - The Docs list (lines 405-419) amends §5 at `:306`, `:309` and
     `:312`. Three more §5 lines are false for a browser and must be
     amended too:
     - `:309`'s last sentence: "after pairing, every connection
       verifies the pin before the token is presented";
     - `:313`: "The client stores the token and the pinned key hash in
       its Keychain";
     - `:317`: "The client verifies the pin on every connection …".

     "A browser … pins nothing" (line 419) needs to name these lines,
     or the spec contradicts itself.
