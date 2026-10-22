# Feedback on the plan for #230, round 2

## Earlier points

1. Settled. The plan adds a random serial, a persisted served leaf under an additive `#[serde(default)]` field, the reissue rule, and the spike's relaunch check.
2. Settled. The plan states the weakening of §5 as the owner's decision, caps a browser pairing at `read`, writes the download fallback, and rewrites §5 lines 309, 312 and 317.
3. Settled. The QR's host now comes from `served.address`, and the app rebinds after `OpenPairing` when the address has moved, with a test for both.
4. Settled. A Mac user agent with `maxTouchPoints > 1` is named "Safari on iPad", and the plan says the heuristic misnames a touch-screen Mac.
5. Settled. The page uses `fetch`'s default mode, `Origin: null` is refused, and a test covers it.
6. Settled. A static request with a cookie goes to the frame and moves its connection to the paired pool, with a pool test. The accept-loop citation in decision 4 should read `:701-705`.
7. Settled.

## Cuts

### Decisions

- **Decision 4: two ways to serve the same three files.** The page is served on the connection thread when the request has no cookie and through the frame when it has one. Since the frame has to serve them anyway, serve all three in `serve_remote`. That is already the plan's written fallback. It removes the `cookie`-dependent branch in `connection` and the "without/with a cookie" paragraph. The new `page` rate window still bounds the frame's load, three files per unpaired visit against 8 drains per pass.
- **Decision 9 and the `web` capability:** cut. No client reads it. The Swift client doesn't, and the page doesn't need to ask its own host. Remove it from decision 9, `remote_info`, the `info_is_served_to_anyone…` test change, and Contract additions.

### The record

- **`ServedCertificate::names`:** cut. Nothing reads it. `served_is_current` compares only `address` and `not_after`, and the SANs other than the address are fixed constants. Drop the field and the doc comment's "`names` or".
- **The list of `Identity` struct literals that gain `served: None`** (`src/adapters/remote.rs:117`, `src/core/tests.rs:11769`, `tests/ui.rs:7436`): cut. The compiler forces each one.
- **The "Rehydration" paragraph:** cut. It restates the `bind_remote` bullet under The actions and effects. Keep only the Known-gaps sentence about a changed address.

### The signals

- **"The existing constructors gain `cookie: None`":** cut. The compiler forces it.

### The tests

- **Core, "a read-only core emits none" for `ServedCertificateMade`:** cut. `update_identity` already withholds `SaveIdentity` when read-only (`src/core/remote.rs:113`), and `bind_remote` returns before issuing anything when read-only (`src/app.rs:2940-2945`).
- **Adapter, "one with `served` round-trips":** cut. `the_served_certificate_is_kept_across_a_restart…` already reloads the stored leaf through the real store. Keep "loads without `served`".
- **"The existing bearer tests and `stalled_strangers_…` pass unchanged":** cut. The suite forces it.
- **`tests/ui.rs`, "and `served: None` elsewhere":** cut. The compiler forces it.

### The UI

- **Rereading `/device` on `visibilitychange`:** cut. The `visibilitychange` handler already reconnects the socket, and `/device` is reread on every `hello`. Keep `hello` and `403 scope`.
