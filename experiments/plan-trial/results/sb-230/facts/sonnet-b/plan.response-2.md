# Response to plan.feedback-2.md

All edits are in `review/1/repo/plan.md` and were re-checked in the file after the edit. Earlier points 1-8 are marked settled, so nothing changed for them beyond the leftover on point 5.

**Leftover on point 5.** Changed: "Where this departs", the `device_name` bullet. "The host needs no new rule" is gone, and the bullet now keeps the cap citation and the control-character filter.

**Cuts, all made**

- "Where this departs":
  - The `addresses` bullet is deleted.
  - The code-in-the-address-bar bullet is deleted.
  - The CSP/WebSocket bullet is deleted.
  - The `last_message` implementer clause is deleted.
- "The record":
  - The schema-version list is shortened. I kept the three numbers in one clause, because the task's standing instruction was to name them as they are today. The "parallel ticket" sentence and the `migrate` sentence are gone.
  - The cookie build bullet under Decision 2 is deleted.
  - Decision 1's SAN/`/cert.der` fallback is one sentence.
- "The signals":
  - The header reasons are dropped.
  - The separate-pool fallback is deleted.
  - The "two tabs" and `MAX_PAIRED` raise sentences are deleted. The queue and the 503 rule stay.
  - The "revoked device" bullet is deleted.
- "The actions and effects":
  - The IPv6 and Crockford remark is deleted.
  - The `build.rs` sentence is deleted.
  - The "struct literal outside the crate" sentence is deleted.
  - The memberwise-init and three-callers sentences are cut to "The memberwise `init` is untouched."
  - The "Copy as JSON" fallback is deleted.
- "The UI":
  - The existing-dialog-test and character-count clause is deleted. "Uses `127.0.0.1` with no LAN address" stays.
  - The `module.exports` mechanism is cut to "exported for Node".
  - The endpoint-table line is one clause.
- "The tests":
  - The adapter-unit header, `HEAD` and 405 items are cut to "`response` for each path and for an unknown one". The same behaviour is still covered under the port tests.
  - The `me` fixture line is deleted.
  - The "not in the pre-commit hook or CI" sentence is deleted.
  - "Not claimed" keeps its first two sentences.
- "Docs, README, changelog":
  - The `docs/remote-client.md` list is collapsed to one line.
  - The `client/README.md` line numbers and the `main.swift:12` reference are replaced by file names.
- "Commit": the closing "No change to `model.rs`…" line is dropped. The contract list now says `AppCore::new_device` drops control characters from a device name.
