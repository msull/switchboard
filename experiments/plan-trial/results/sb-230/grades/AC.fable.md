| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| AC | 3 | 2 | 1 | 2 | note |

## Grounding
- `src/adapters/remote.rs:495–503` (read_request hands the query to the frame): found (the `target.split_once('?')` and query collection are exactly lines 495–503; the `RemoteRequest` literal follows at 506)
- `src/core/remote.rs:65` (`Paired` closes the window): found (line 65 is `AppAction::Paired(device)`, with the comment that the code is single-use and `self.pairing = None`)
- `docs/remote-client.md:207` (card excerpt described as `last_message` from the Stop hook): found (line 207 is the `session.last_response` bullet and ends with exactly that sentence)
- `src/core/events.rs:127–135` (stores only `last_stop_at` and `pending_at_stop`): found (the `Stopped` arm sets those two fields and nothing resembling an excerpt)
- `control/src/lib.rs:637–689` (`SessionView` has no excerpt): found (struct spans 637–688; `card`, `waiting`, `waiting_reason`, `last_activity_ms`, `prompt_refusal` present, no excerpt field)
- Spot-checks beyond the five also held: `app.rs:3142–3149` 401 refuse, `ui/remote.rs:157–164` payload, `remote.rs:101–106` SAN-less params, `RemoteReply` 388, `respond` 530, pools 567–568, `DRAIN_PER_PASS` 580, `der_element` 268, `action.rs:248` `RevealDocument`, `action.rs:1248` routing, §4.4 item 3 "read in the last minute" watch rule, `tests/remote.rs:3226`, `tests/ui.rs:7474–7490`.

## Departures caught
- WebSocket will not open from Safari: caught. Polling the REST routes is the base, the socket an enhancement; acceptance met by polling.
- Certificate download is not an option with this certificate: caught with a wrong fix. The plan sees the SAN/validity problem but answers it by reissuing the certificate with IP SANs, persisting it in `identity.json`, and adding an export-to-AirDrop path, which ties the identity to an address against §5/§9. It flags the cost under "Left for the owner" but builds it anyway.
- Each file is a connection: caught. Page requests counted against the 4-slot pool, `Connection: close` frees the slot, and a five-connection measurement is in the spike. Three files rather than two, which `Connection: close` makes tolerable.
- Device name is the page's to send: caught (`device_name` from `navigator.userAgent`, no host parsing).
- Beyond the reference: the excerpt is not on the wire (caught, with the spec line to correct); no way for a device to read its scope (caught, `/me`); invite carries a LAN address when listening on loopback (caught); pair reply would carry the token in the body even with a cookie (caught, body omits `token`); revoked and unknown tokens are the same 401 (caught, `/info` inference); the key in the URL is not a defence against a TLS-terminating relay (a real point the reference does not make); the query form puts the code in the first request line (fragment form instead, a sound improvement).

## Decisions to overturn
- Reissuing the certificate with IP SANs and persisting it (with `CertificateReissued`, the mirror action and the `bind_remote` logic). Reference decision 1: click-through is the trust step, no reissue; the SAN-less certificate is still click-through-able and the identity model deliberately binds no address.
- The certificate export file (`ExportCertificate` action and effect, `save_certificate_file` port method, AirDrop instructions, dialog button). Reference decision 1: no certificate download.
- Lesser, not counted: a fixed cookie name `__Host-sb` rather than one carrying the server id; two instances on one Mac (a dev data dir on another port) would overwrite each other's cookie, which the reference calls out and the plan's own "not scoped by port" note nearly reaches.
- The reference's `watch` frames: the plan's reliance on the one-minute read window plus a 20 s poll is consistent with §4.4 item 3; I would not reverse it.

## Over-specification
- The whole certificate thread (reissue, DER SAN reader, export effect, its tests, the owner branches about trusting the file) is roughly a fifth of the plan and should not be built.
- `Limits.page` at 120/min with its own tests, the `node --test` harness for `app.js` wrapped in a Rust test, and `no-store`/`nosniff` on every JSON reply are extras the suite would not demand. The contract-additions list repeats the body. Around a quarter to cut overall.

## Hand-off
- Drop the certificate reissue and the export file: the click-through is the trust step, said in the dialog and §5; keep everything else, including the fragment URL, `/me`, the cookie, and poll-first.

## Notes
- best: Caught every departure the reference lists except one, plus five it does not, each with the line that proves it and a fix the host can keep honest (the `/info` inference for "revoked", the fragment URL, the token kept out of the pair body).
- worst: Built the certificate reissue and download path that the facts file's unresolved items scared it into, against the spec's identity model, when the click-through alone would do.