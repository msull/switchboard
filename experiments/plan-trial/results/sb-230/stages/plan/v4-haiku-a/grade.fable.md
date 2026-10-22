| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| v4-haiku-a | 3 | 2 | 1 | 2 | note |

## Grounding
- `docs/remote-client.md:207`: found (the `session.last_response` bullet, ending with the sentence that the card excerpt is `last_message` "from the `Stop` hook on the record").
- `src/ports/events.rs:24`: found (`last_message: Option<String>` is a field of `EventKind::Stopped`).
- `src/adapters/hooks.rs:71`: found (`last_message: Option<String>` on the hook line struct, passed into the event at line 196).
- `src/core/events.rs:132`: found (`EventKind::Stopped { pending, .. }` sets `last_stop_at` and `pending_at_stop` and drops the message; no record field carries it, so the plan's correction of facts line 13 is right and the facts file was wrong).
- `control/src/lib.rs:637`: found (`SessionView` with `card`, `waiting`, `waiting_reason`, `last_stop_at_ms`, `trust_question`, `prompt_refusal`; no excerpt field).

## Departures caught
- Safari refuses `wss://` after a click-through: caught. Polling every 3 s is the default, the socket is an enhancement with `watch` on the open key, and failure degrades to polling with a message.
- Certificate download is not an option with this certificate: caught with a wrong fix. The plan sees the certificate meets none of Apple's rules, then builds the reissue (SAN, EKU, 800 days) plus `/host.cer` and profile instructions, which is the identity-to-address binding the reference says the spec avoids.
- Each page file is a connection: caught. Static replies are answered on the connection thread with `Connection: close`, and a starvation test is listed. The page is three files plus `/host.cer` rather than the two the reference wants.
- Device name is the page's to send: caught. `device_name` from `navigator.userAgent`, capped by `new_device`.
- Found beyond the reference: the excerpt is not on the wire (verified, and the plan correctly overrides the facts file); scope is on no endpoint after reload (adds `/api/v1/device`); the pair reply would carry the token in its body (omitted when `browser: true`); the URL key does not stop a relay for a browser (amends `remote-client.md:309`); "revoked" is indistinguishable from "never paired"; the URL may be in history before `replaceState` runs.
- Missed: the invite carries a LAN address while the host may listen on loopback.

## Decisions to overturn
- Reissuing the certificate at bind with SAN, `serverAuth`, 800-day validity, the `cert_names`/`cert_issued` identity fields, the schema bump, `AppAction::IdentityReissued`, and spike 23. Reference decision 1: no reissue; the click-through is the whole trust step.
- Serving `/host.cer` and telling the owner in the devices dialog to install it as a profile. Reference decision 1 and departure 2: no certificate download.
- I agree with the reference on the rest. The cookie named by server id with `SameSite=Strict`, the Origin check, the static arm on the connection thread, one URL QR with JSON still parsed, `/device` for scope after reload, and the last-response polling for the excerpt all line up.

## Over-specification
- The reissue machinery (identity record changes, action, bind-time logic, spike, its three tests, the profile-reinstall follow-up) is unneeded under the reference and is about a fifth of the plan.
- The facts-count correction (15 vs 10 `RemoteReply` sites), the verbatim dialog paragraphs, the long owner follow-ups list, and the "schema versions unchanged" lines add perhaps another tenth. Roughly a quarter to cut.

## Hand-off
- Drop the certificate reissue and `/host.cer` entirely; the click-through alone is the trust step, and everything else in the plan stands.

## Notes
- best: verified that `last_message` lives only on the Stop event and is dropped by the core, contradicting its own facts file correctly, and shipped a polling-first page with a real cookie transport and `/device`.
- worst: turned "the certificate as built cannot be trusted" into a reissue-and-download subsystem the spec's identity model deliberately avoids, with a schema bump and a spike to carry it.