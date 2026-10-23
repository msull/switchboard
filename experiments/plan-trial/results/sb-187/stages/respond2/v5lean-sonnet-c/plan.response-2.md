# Response to round 2

Every cut was made. Edits are in `plan.md`.

## Cuts

### Where this departs from the issue
- Bullet 1: cut to one sentence plus the `fail_attempt_with` citation. The `app.rs`/`scheduler.rs`/`restart.rs`/`recover.rs` citations are gone. Changed: Where this departs.
- Bullet 2: "No new storage" and the pointer explanation are gone. Changed: Where this departs.
- Last paragraph: dropped. Changed: Where this departs.

### The record and The signals
- Both sections deleted. The one line is under Contract additions ("no record, signal, `AppAction` or `Effect` change; no schema bump"). The `cleanupPeriodDays` parenthetical went with the Signals section.

### The actions and effects
- Item 1, `Cancelled`: now "`Failed` only; a `Cancelled` copy may be one Dispatch discarded (`ticket.rs:228-232`)".
- Item 1, unverified premise: two sentences, the half-edited-copy risk and the alternative dropping `revise` on every chain.
- Item 1, struct: dropped. `review_subject` keeps its `(PathBuf, String)` return; only the path choice changes. The ambiguous `revised` rule is gone.
- Item 2: appends only ` The next attempt reviews <path>`. The two-variant wording and "A question already asked keeps its old text" are gone.
- Item 3: kept.

### The UI
- Section deleted, with the `LaneFile` fields, the serde round-trip test, the `print_lane_files` notes, the `serve.rs` change and the "no change" bullet. `shown_where` (item 3) covers `show` and the ticket page.

### The tests
- (e) dropped. (c2) merged into the chained-failure test in `first_slice.rs`, so one chain test remains. The `serve.rs`, `dispatch_control` and `main.rs` bullets are gone. "none sleeps" is gone.

### Docs
- `design.md`: I opened the file. The known-gaps list (line 3098) does not list this gap, so the plan now says to add one status sentence to the rerun paragraph at line 1702.
- README line and "app runs at every step" dropped.
- `docs/dispatch.md`: test names dropped.

### Follow-ups for the owner
- The section is removed. All three bullets are gone: the first two duplicated "Where this departs" and the facts file, and the third was about Codex `resume`, which this plan does not use.

### Contract additions
- Now `Attempt::holds_review_copy` (internal) plus the one line. Both `LaneFile` fields are gone.
