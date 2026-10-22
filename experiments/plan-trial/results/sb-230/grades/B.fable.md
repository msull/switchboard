| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| B | 3 | 1 | 1 | 2 | note |

## Grounding
- `control/src/lib.rs:637` (`SessionView`, no excerpt field): found (struct at 637-638; no `last_message` or excerpt field anywhere in `control/src/lib.rs` or `src/core/model.rs`, so the departure is real)
- `src/ui/cards.rs:558` (`agent_excerpt`): found (fn at 558, derives the first line from the parsed `Conversation`)
- `docs/remote-client.md` §4.3 `last_message`: found (line 207 says the card excerpt is `last_message` on the session's metadata, and it was never added to the view)
- `src/app.rs:3023` (allowlist prefix match): found (`strip_prefix("/api/v1/")` at 3024 inside `serve_remote` at 3017)
- `src/adapters/remote.rs:567` (`MAX_UNAUTHENTICATED`): found (exactly, with `MAX_PAIRED` on 568 and the windows at 583-584)
- Also checked: `src/ui/remote.rs:160` payload, `make_identity` at 95-102 with `CertificateParams::new(Vec::<String>::new())` (no SANs, as claimed), `main.swift:111`, `BrowserView.swift:106` and `:883`, `src/app.rs:486` revocation, `pairing` at `:141`, `tests/ui.rs:7424`. All found.

## Departures caught
- WebSocket will not open from Safari: missed. Step 4 is "Subscribe first ... wait for `hello`, then GET"; the list never loads on the device the issue is for. Only the session view has a 30 s refetch.
- Certificate download is not an option: caught, with the right reasons (no SAN, IP address rejected even as a trusted root, address in the certificate ties identity to a DHCP lease, iOS profile install).
- Each file of the page is a connection: missed. The plan notes "a page load is three requests" and keeps three files on a pool of four, without saying what the idle keep-alive connections leave for a second device.
- Device name is the page's to send: caught (decision 6, `navigator.userAgent` filled into `device_name`).
- Found beyond the reference: `SessionView` carries no excerpt and the spec's `last_message` was never added, so the page reads `last-response` for waiting Claude Code sessions instead. Verified and correct.
- Found beyond the reference: the `/pair` reply body would still carry the token even with a cookie; the plan omits it under `transport: "cookie"`. Sound.

## Decisions to overturn
- Socket-first live updates with no polling for the list (reference decision 3: poll first, socket second). On Safari the page would sit on "waiting for hello".
- Three static files (`/`, `/app.css`, `/app.js`) on an unchanged pool of four (reference decision 5: at most two files, or raise `MAX_UNAUTHENTICATED`). Fold the CSS into the HTML.
- Not reversed but noted: the fragment instead of the query differs from the issue and reference. The `Referer` argument is moot because the plan's own `Referrer-Policy: no-referrer` covers the script fetch, but keeping the code out of the request line is a fair reason and the phone's camera opens it the same way. I would let it stand.
- Not reversed: the `Origin` check, `wrong_origin`, cookie clearing on `401`, and the `web` capability are extras the reference calls fine.

## Over-specification
- "The record", "The signals", "The actions and effects" are three sections that say nothing changes; the Commit section restates the contract list already given under "The port"; the exact hint and dialog sentences are prose an implementer would rewrite. Roughly 15 percent.

## Hand-off
- Safari refuses `wss://` to a click-through certificate: load and refresh the list and session by polling, use the socket only when it opens, and fold `app.css` into the HTML so a page load holds two unauthenticated slots, not three.

## Notes
- best: the excerpt departure, verified against `SessionView` and the spec's unbuilt `last_message`, and the SAN/DHCP reasoning that collapses the certificate-download option.
- worst: a subscribe-first page on a socket the target browser will not open, with no polling behind it.