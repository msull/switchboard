# Response to review 1

1. **The cookie leaks to other services and can be replayed.** Accepted,
   with the split credential.
   - The token is now two halves:
     - the `HttpOnly` cookie;
     - a key in `localStorage`, which is port-scoped. It travels as
       `X-Switchboard-Key`, and on the WebSocket as an `sbk.<key>`
       subprotocol beside `switchboard.v1`, which the 101 echoes.
   - `token_sha256` hashes both halves together, so the device record
     does not change and a cookie alone is useless.
   - Dropped as unnecessary: the Origin-and-header rule,
     `X-Switchboard-Web` and `cross_origin`.
   - Cookie tossing can only make requests fail, and the page says to
     pair again.
   - The cost: Safari clears script-written storage after seven days
     without a visit. A browser that loses the key gets a new
     `key_missing` 401, and the page explains it.
   - Scope stays what the owner chose (`read` by default), since the
     leaked half carries none.
   - Changed:
     - Where this departs ("A cookie alone cannot be the credential");
     - Decision 2;
     - The record (On the device);
     - The signals (`RemoteRequest` fields, the Authentication table, the
       new requests);
     - Actions and effects (`respond_upgrade`, `authenticate`,
       `remote_pair`);
     - The page (Load step 5);
     - The tests (`half_a_browser_token_is_refused`, the pairing test,
       the upgrade unit test);
     - the spike's subprotocol item;
     - Docs (§5 rewrite, known gaps);
     - Contract additions.

2. **An address change silently unpairs the browser.** Accepted.
   - The invite host is `<hostname -s>.local`, the same host name the
     Bonjour record already advertises (`src/adapters/bonjour.rs:41`). The
     LAN IP is used only when there is no such name.
   - The certificate gains that name as a SAN.
   - The spike checks that the name loads in all three browsers. If it
     does not, the fallback is the LAN IP plus a named gap.
   - What remains, a renamed Mac or a lost key, is named as "Known gap: a
     stale device record" in The record. That section says the dialog
     shows the old entry with its "seen <time>" until the owner revokes
     it.
   - Changed:
     - Where this departs;
     - Decision 3;
     - The signals (the certificate's SANs, `local_host_name`);
     - The tests (the spike's `.local` item);
     - Docs (known gaps).

3. **The QR cache goes stale, and the no-address case is undefined.**
   Accepted.
   - The texture cache is keyed on the payload string.
   - With `lan` on and neither a `.local` name nor a LAN address, no QR is
     drawn and "Copy invite" is hidden, because a URL with no host opens
     nothing. The window says why. The key, the code and "Copy JSON
     invite" stay.
   - The UI test ticks the local-network box with the window open and
     asserts the copied URL's host changes. It also says the texture key
     itself cannot be observed from the harness.
   - Changed: The UI (the dialog), and The tests (`tests/ui.rs`).

4. **The URL invite breaks older clients.** Accepted, by keeping the
   JSON.
   - A "Copy JSON invite" button produces today's text, so an older
     `switchboard-remote` or headset build still pairs by paste.
   - The CHANGELOG line and §5 name it.
   - Changed:
     - Where this departs ("The invite stays readable by clients built
       before this change");
     - Decision 3;
     - The UI (the dialog);
     - The tests (`tests/ui.rs`);
     - Docs (§5, README, CHANGELOG).

5. **Renewal only at startup.** Accepted.
   - Renewal goes through a new `AppAction::CertificateRenewed`, so the
     core's `update_identity` emits the existing `SaveIdentity`.
   - `renew_remote_certificate(now)` runs in `listen_remote` before the
     wake is set, then daily from `Tick`. With the 30-day margin, a
     running app never reaches `not_after`. The due check reads only the
     public certificate; the Keychain is read only when renewing.
   - The `SaveIdentity` arm of `run_remote_effect` now rebinds when the
     certificate differs from the one bound. `bind_remote` builds a fresh
     `tls_config`.
   - The "replaced once on the first listen" wording is gone.
   - The dialog shows the certificate's issue date, and the trust
     sentence names the one expected warning after a renewal.
   - New test: `a_certificate_that_comes_due_while_running_is_renewed`.
   - Changed:
     - Decision 1;
     - The record;
     - Actions and effects (the action, `renew_remote_certificate`,
       `run_remote_effect`);
     - The tests (the core test and `tests/remote.rs`);
     - Contract additions.

6. **The §5 sentences would be false.** Accepted.
   - Docs in the same commit now gives replacement text for:
     - §5 line 309 (pin before the token);
     - §5 line 312 (bearer only, never in a URL);
     - §5 line 318 (out of URLs);
     - the design.md summary at line 3111.
   - A correction: line 318 as written today says only tokens are kept
     out of URLs, not codes. The plan still adds a sentence on how the
     code in the invite URL is handled.
   - Changed: Docs in the same commit.

7. **Icon probes go through the frame and the pool.** Accepted.
   - Every path outside `/api/v1/` is answered on the connection thread,
     with `Connection: close`, and counted in the page window: the four
     files, or a 404 for `/favicon.ico`, `/apple-touch-icon*.png` and the
     rest. The frame's prefix match stays as a backstop.
   - The spike's pool measurement counts every request, probes included,
     from a fresh profile and from a home-screen launch.
   - The static-path test asserts the probes never reach the inbox.
   - Changed:
     - Where this departs;
     - Decision 4;
     - Actions and effects (`connection`, `web::answer`, `Limits`);
     - The tests;
     - Docs (§4.1).
