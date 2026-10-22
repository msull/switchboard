| plan | grounding | departures caught | decisions to overturn | over-specification | hand-off |
|---|---|---|---|---|---|
| I | 3 | 1 | 0 | 0 | no |

## Grounding
- `src/app.rs:3568`: found (`remote_info` returns the identity’s `spki_sha256` in JSON).
- `docs/remote-client.md` §5: found (requires key verification before sending the code; describes pairing, bearer tokens and revocation).
- `design.md's "Trust boundary"`: found (in `docs/design.md`; covers execution approval and secrets, but the quoted key-verification rule is in the protocol spec).
- `src/ui/cards.rs:558`: found (`agent_excerpt` derives a capped excerpt from the conversation’s last turn).
- `:178`: found (in `src/ui/cards.rs`; calls `agent_excerpt` for agent cards).

## Departures caught
- Safari cannot open the click-through certificate’s WebSocket: missed. A measurement gate replaces the required polling fallback and can stop delivery entirely.
- Certificate download is unsuitable with the existing certificate: missed. Chooses click-through, but omits the SAN/EKU/validity incompatibilities and permits conditional SAN reissuance.
- Parallel static fetches consume scarce unauthenticated connections: caught with a wrong fix. Keeps three files and four slots, assumes keep-alive suffices, and leaves no concrete capacity for a second device.
- Device name must come from page script: caught; uses `navigator.userAgent` to populate `device_name`.
- Additional findings: the listing lacks the card excerpt; reloads need a way to recover device scope; loopback binding can make the advertised LAN QR unreachable. These are useful observations absent from the reference.
- The claimed additional trust departure overstates disagreement with the issue: the issue already acknowledges impostor-served pages. The reference retains the URL key’s value against relayed codes.

## Decisions to overturn
- Replace socket-only updates and the blocking Safari spike with polling first and opportunistic WebSockets, per reference decision 3.
- Remove the extra owner-sign-off gate and conditional certificate reissue; reference decision 1 settles click-through without reissue.
- Use two static files and explicit second-device capacity, per reference decision 5.
- Make “Copy invite” copy the URL; retain JSON parsing compatibility rather than adding a separate copying workflow, per reference decision 4.
- Restore the requested excerpt rather than unilaterally deferring it, per reference decision 6. The discovered missing field is valid evidence that implementation needs further design.

## Over-specification
- Roughly half is removable: repeated trust arguments and gates, exact cookie-name mechanics, constructor counts, hypothetical schema migration instructions, exhaustive source-text assertions, and a commit-body restatement. The elaborate browser-header policy also exceeds the reference’s SameSite defence plus optional Origin check.

## Hand-off
- no: The plan lacks the reference’s essential Safari polling fallback and makes delivery conditional on a WebSocket capability the reference says fails.

## Notes
- best: Precise repository investigation identifies missing excerpt data and provides concrete cookie, revocation and native-compatibility tests.
- worst: Extensive security and implementation detail obscures the missing mechanism that makes live updates work on the target phone.