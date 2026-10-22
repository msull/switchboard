1. **The card excerpt (plan lines 13-26): the plan changes a decision the spec already made, and the premise it gives in support is false in the tree.**
   - **What the spec says.** `docs/remote-client.md:207` says what the excerpt is: "The compact card's one-line excerpt is not part of it [`session.last_response`]: that is `last_message` from the `Stop` hook on the record, a field of the session's metadata, and moves with `changed {session}`." Line 201 says it is masked before it is cut.
   - **What the plan does.** It swaps that excerpt for a per-session `GET sessions/{id}/last-response`, which is a projection of the `conversation` key, and so changes the excerpt's identity and the event that refreshes it. The issue asks for "the one-line excerpt the card shows". It does not ask to change what that excerpt is.
   - **The masking concern holds.** The tree supports it: the policy is resolved on the worker, not the frame (`src/app.rs:1822-1826`, `docs/remote-client.md:463`). The answer to it is to mask `last_message` the way the spec requires. Redefining the excerpt does not answer it.
   - **The supporting premise is false.** The plan says "No `watch` is registered for these, so the 128-key watch set is not touched by the list." In the tree, every content read calls `touch_watch` (`src/app.rs:2111`). That inserts the key into `self.watches` for a minute (`src/app.rs:1754-1768`), and past `WATCH_CAP` the least recently read keys are evicted (`:1789-1798`). So a page polling a list of Claude sessions does fill the shared watch set and can evict a headset's keys.
   - **The cost is also missing.** Each first fetch is a whole-transcript parse on the one content worker (§4.1, where Spike 22 measured a 575 MB peak). The plan does not count that.
   - **What to do.** Either carry `last_message`, masked, as §4.3 says, or ship rows without an excerpt and name that as a gap. Do not redefine the excerpt.

2. **The test "every `/api/v1/…` literal in `app.js` (a `${…}` replaced by a UUID) answers anything but `404 not_found`" (plan lines 413-415) contradicts the tree.**
   - For an unknown id, `content_job` returns `(404, "not_found", "no such session")` (`src/app.rs:1856-1861`), and `remote_content` passes it on as a refusal (`:2093-2097`). So `sessions/<random uuid>/last-response` and `…/screen` answer exactly `404 not_found`.
   - As written, the test either fails or has to exempt the paths it exists to check. It needs seeded ids, or it needs to tell the allowlist's `"not available on this port"` refusal (`src/app.rs:3108-3114`) apart from a missing-record one.

3. **Two line references do not match the worktree at 7fe8949.**
   - `bind_remote` is at `src/app.rs:2936`. The plan says `:2935` (line 348).
   - `Limits` is `src/adapters/remote.rs:623-639`. The plan says `:625-640` (line 262).
   - The others I checked hold: `:360`, `:389`, `:444-513`, `:519-527`, `:770`, `:95`, `:139,146`, `app.rs:3017`, `:3023`, `:3127-3153`, `:3568`, `:3606`, `:3754`, `core/remote.rs:46-72`, `:105-117`, `:166-175`, `ports/remote.rs:17`, `:26-42`, `:96-107`, `model.rs:12`, `:612`, `tests/remote.rs:245`, `:2892`, `:3226`, `tests/ui.rs:7424`, `Info.swift:114`.

4. **The `IDENTITY_SCHEMA_VERSION` bump is never written for an existing installation.**
   - The plan sends `CertificateReissued` "through the existing `update_identity`" (plan lines 336-339). `update_identity` clones the identity and changes only what its closure changes (`src/core/remote.rs:105-117`). The only place that sets `schema_version` is `make_identity` (`src/adapters/remote.rs:118`).
   - So an upgraded `identity.json` gains `certificate_names` and `certificate_expires` but keeps `schema_version: 1`. The CLAUDE.md rule ("a schema bump for any serialized change") is then met only for fresh identities.
   - The `CertificateReissued` arm has to set `schema_version` to the new constant, and the core test the plan names (plan lines 396-397) should assert it.
