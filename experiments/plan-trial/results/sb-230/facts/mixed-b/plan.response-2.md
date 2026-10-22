# Response to feedback round 2

Earlier points 1–7 were marked settled; nothing changed for them. All
eleven cuts were made in `review/1/repo/plan.md` only.

- **Departs from the issue:** the "Keeps the token" bullet and the
  "native text path" bullet are removed. Decisions 2 and 3 carry both.
- **Decision 5:** now "embedded with `include_str!`, no build step, no
  dependency, no `build.rs`. Tests below." The `[package] include` and
  crates.io sentence is gone.
- **Decision 6:** removed. The Record section says the same.
- **The record:** the parallel-ticket clause is removed. It ends at "No
  serialized type changes, so none bumps."
- **The signals:** the `RemoteReply` literal line list is removed. The
  bullet says only that `RemoteReply` gains `set_cookie`.
- **The page, step 6:** the op id is 16 bytes from
  `crypto.getRandomValues`, hex. `crypto.randomUUID()` appears only in a
  parenthesis saying the plan does not depend on it.
- **Tests, `tests/remote.rs` case 1:** the instruction to amend the two
  allowlist tests is removed.
- **Case 6:** the `stalled_strangers_…` sentence is removed.
- **Case 7:** the second case that replays the queue by hand is removed.
- **Case 2:** the `MAX_IN_FLIGHT = 2` and `new WebSocket` source-text
  checks are removed. The `mode:` check stays, because Origin
  correctness depends on it.
- **"No Swift test or fixture changes" bullet:** removed.
