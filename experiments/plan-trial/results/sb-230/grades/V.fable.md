| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| V | 3 | 2 | 1 | 2 | note |

## Grounding
- `src/adapters/remote.rs` 496–502 (`read_request` splits the query into `RemoteRequest.query`): found (lines 495–503 split the target on `?` and collect the pairs; `read_request` starts at 444 as the plan says elsewhere)
- `docs/remote-client.md` 302–320 (spec §5, "the code alone would let an intermediary … take the token"): found (§5 heading at 302, the pairing bullet at 309, the first bullet "before anything but `/info` and `/pair`" at 306, the token bullet at 312)
- `control/src/lib.rs` 645–646 (`SessionView.card` is the card's word): found (doc comment and field exactly there)
- `src/ui/cards.rs` 197 (`waiting_reason` as the card's second line): found (`cx.core.waiting_reason(record.id)` at 197, gated on `WaitingOnYou`)
- `src/app.rs` 3166–3173 (403 with code `scope`, "this device reads only; raise its scope on the host"): found (also 3127–3129 `authenticate` reading only `request.bearer`, and 3146 the "token is not known" 401, as cited)

Also verified: `connection` at 770 with `limited` at 788 and the frame queue after; `respond` at 530; `MAX_UNAUTHENTICATED` at 567; the JSON payload at `src/ui/remote.rs` 160–165 and the LAN checkbox at 110–112; `local_address` at 962.

## Departures caught
- Safari refuses `wss://` to a click-through certificate: missed. The plan lists "`wss://` covered by the click-through" as a run item, defaults to "assumed", and its fallback ("the page says accept the warning again") does not work. Live updates are socket-only with no polling, so the page is static on the device it is for.
- Certificate download is not an option with this certificate: missed. The plan keeps "install the Mac's certificate as a trusted profile" as the owner's fallback and a SAN reissue as a run-item fallback, treating both as viable.
- Each file is a connection: caught, with a reasonable fix (`Connection: close` on static replies, a pool-turnover test), though it serves three files where the reference wants two, and it does not say what the pool leaves for a second device.
- Device name is the page's to send: caught ("the host does not parse a user agent").
- Beyond the reference: the excerpt is not on the wire (shows `waiting_reason` or `card`); no route returns a device's own scope (stored from the pair reply, corrected by the 403); the pair reply would carry the token in the body (a `via: browser` marker strips it); the invite would carry a LAN address while the host listens on loopback (URL shown only with LAN on); revoked and rotated are the same 401 (distinguished by refetching `/info`); and the query string puts the code in the request line the host reads, so the plan moves it to the fragment. That last one the reference did not consider and is a genuine improvement.
- The plan's departure 5 (the key in the URL does not defeat an impostor who relays `/pair`) is sharper than the reference's "keeps its value against a relayed code"; I side with the plan: the key is in the attacker's hands, so it only catches a different Mac at the address.

## Decisions to overturn
- Socket-only live path. Reference: poll first, socket when it opens. Without polling the acceptance step "a waiting session moves to the top without a reload" fails on an iPhone.
- Cookie named `__Host-sb`. Reference: a cookie named with the server id, because cookies ignore ports and two instances on one Mac would overwrite each other. The plan notes port non-isolation as an owner cost but keeps a fixed name.
- Not counted: three files instead of two (inlining the CSS is a one-line change and `Connection: close` already covers the pool concern); the per-response `wss://<Host>` CSP with a 400 on a bad `Host` is heavier than the reference's `connect-src 'self'` but not wrong.

## Over-specification
- Roughly a quarter. The "Record" section says that no schema version changes; "No CLAUDE.md edit", "Contract additions: none" and the commit message say nothing an implementer needs; the ten-row Unresolved table repeats the facts file's open items with their defaults already stated in the body; the `Host` validation and per-response CSP builder add a mechanism the plan then has to test ("Bad Host") for a header `'self'` would cover.

## Hand-off
- Make polling of the collections and the open session the primary live path and use the socket only when it opens, since Safari refuses `wss://` to a click-through certificate; and name the cookie by server id.

## Notes
- best: the fragment instead of the query, with the RFC 9112 reason, so the code never reaches the host's request line, and the revoked-versus-rotated distinction resolved by comparing the stored key with `/info`.
- worst: assuming the WebSocket works after the click-through and building no polling, which leaves the first cut static on the phone the issue is for.