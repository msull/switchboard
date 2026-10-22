# Response to plan feedback, round 1

1. **Watch set.** Accepted.
   - Where this departs from the issue, the excerpt bullet: excerpts are
     now bounded to the first 8 waiting Claude Code rows. They are
     fetched only at load and on that session's own `changed session`.
     The bullet states the cost, citing `docs/remote-client.md:254` and
     the facts' For the owner line "Card excerpt": at most 9 of the 128
     keys.
   - The UI, the page:
     - the list row and the `changed session` bullet carry the bound,
       and while polling an excerpt is refetched only when
       `last_stop_at_ms` moves;
     - "at most one watched key" is replaced by the 9-key account and
       what a headset loses if the 128 are passed.
   - Docs, the design.md bullet: it names the watch-set cost and the
     8-row gap.

2. **A 401 on the upgrade is invisible.** Accepted.
   - The actions and effects, 1 (Trust), the fallback bullet: it cites
     WHATWG WebSockets §2.2 and §4. After any socket failure or close,
     the page makes one `GET device`:
     - 401 is the revoked state;
     - a network error backs off and counts no failure;
     - 200 counts one failure toward the fallback.
   - While polling, the page retries the socket once a minute and returns
     to live on `hello`.
   - The UI, the page: the reconnect bullet is reworded. The revoked
     state comes from that `GET device` or any `fetch` 401, never from
     the upgrade.
   - The tests: step 6 of the end-to-end test is reworded to match.

3. **The 404's `Connection` header.** Accepted.
   - The actions and effects, 4 (Static paths): both the page files and
     the 404 go through a new private `respond_closing`, which writes
     `Connection: close`. The 404 carries `application/json` and no page
     headers.
   - The tests: the `/favicon.ico` and `POST /` assertion now names
     exactly those headers instead of "the same way".

4. **The refusal log.** Accepted; it keeps the log.
   - The actions and effects, 4: the 404 on the connection thread logs
     `remote: {method} {path} refused: not_found` as `refuse` does today
     (`src/app.rs:3762-3766`). The log line has the method and path
     only.
   - The query never reaches it: `RemoteRequest::path` excludes the
     query (`src/adapters/remote.rs:362-363`), so the code in
     `/?code=` is never logged.
   - The sentence "Nothing on the connection thread logs a request" is
     removed.
   - Docs, §4.1: the log rule is added there.

5. **CSP `'self'` and `wss`.** Accepted. In the actions and effects, 4,
   "unverified" is replaced:
   - CSP Level 3 §6.7.2.8 and §1.3 are cited;
   - so is WebKit bug 235873 (FIXED 2022-04-02, r292266), with 201591
     as its duplicate;
   - the `wss://<Host>` fallback is now said to cover only a WebKit
     older than that, and a bare `wss:` is ruled out because it allows
     any host.

6. **Premises cited as Settled, and the §5 lines.** Accepted.
   - The actions and effects, 2 (Token transport):
     - "Settled" now covers only the per-origin fact;
     - the secure-context claim cites the HTML standard's
       `WindowLocalStorage` IDL;
     - a `SecurityError` from the getter keeps the token in memory, and
       the page says "This browser will not keep the pairing; it lasts
       while this tab is open."
   - Docs, the §5 list: `:309`'s last sentence, `:313` and `:317` are
     now named. Each sentence gains that a browser relies on its
     certificate exception and checks `/info`'s key against what it
     stored.
