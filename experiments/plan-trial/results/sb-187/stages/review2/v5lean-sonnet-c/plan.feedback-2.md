## Earlier points

1. Cancelled attempts: closed. `holds_review_copy` is `Failed` only, with test (c).
2. Wrong line reference: closed. The `app.rs`, `scheduler.rs` and `workflow.rs` citations replace it.
3. Unsupported "newest wins" premise: closed. The plan states it as unverified, accepts the risk, and gives the chain case as the reason.

## Cuts

### Where this departs from the issue
- Bullet 1: keep one sentence ("the plan does not depend on why the run vanished; it reads the failed attempt's record"). Drop the `app.rs`/`scheduler.rs`/`restart.rs`/`recover.rs` line citations. They restate the facts file and no implementer acts on them.
- Bullet 2: drop "No new storage" and the "pointer" explanation. "The record" already says it.
- Last paragraph ("Everything else in the issue is as the code does it"): drop it. It restates what the Actions section does.

### The record and The signals
- Delete both sections. Each says "Nothing". Put one line under Contract additions: "no record, signal, `AppAction` or `Effect` change; no schema bump".
- Drop the `cleanupPeriodDays` parenthetical. The plan says the behaviour is unchanged.

### The actions and effects
- Item 1, `Cancelled` bullet: cut it to "`Failed` only; a `Cancelled` copy may be one Dispatch discarded (`ticket.rs:228-232`)".
- Item 1, unverified-premise bullet: cut it to two sentences. State the risk of a half-edited copy, and that the alternative drops `revise` on every chain.
- Item 1, struct: do not introduce `{ path, session, from, revised }`. Keep the tuple, or add a `path` field. Only `path` and `session` are needed once the question just names the path. The `revised` rule ("or its `n` differs from…") is also ambiguous.
- Item 2: drop the two-variant wording ("revised after your note" versus "the plan stage's output"). Append the path only. Drop "A question already asked keeps its old text".
- Item 3: keep it. The `holds_review_copy` predicate is the one shared piece.

### The UI
- Delete the section. `shown_where` (item 3) already makes `show` and the ticket page name the copy the rerun reviews.
- The `LaneFile.review_failed` and `earlier` fields, the serde-compat round-trip test, the `print_lane_files` notes and the `serve.rs` change are an enhancement built before the default has been seen to fall short. Nothing in the issue or facts asks for them.
- Drop the "Ticket page: no change" bullet.

### The tests
- Drop (e). The `Failed`-only predicate forces it.
- Merge (c2) into the second `first_slice.rs` test (the chained failure). Keep one chain test.
- Drop the `serve.rs`, `dispatch_control` and `main.rs` test bullets with the UI section.
- Drop "none sleeps". `CLAUDE.md` already says it.

### Docs
- `design.md`: drop "if it is listed; otherwise one status line". Say which it is once the file is open.
- Drop the README line and "The app runs at every step; no `src/` change".
- `docs/dispatch.md`: drop "the new test names".

### Follow-ups for the owner
- Drop the first two bullets. "Where this departs" and the facts file already cover them.
- Drop the Codex `resume` bullet. The plan resumes no reviewer.

### Contract additions
- Cut to `Attempt::holds_review_copy` (internal) plus the one line above. Remove both `LaneFile` fields with the UI section.
