# Response to feedback 1

1. **Parked tickets.** Changed. The new rule excludes a parked ticket: `Ticket::carries_revision` is false while the ticket's state is `Parking` or `Parked` (section "Actions and effects", item 1). So `shown` and `review_copy` return `plan/1` for a parked ticket, matching the comment at `ticket.rs:1862`. The "Tests" section adds the parked case. A new "Decisions for the owner" item 3 says so and says that resuming and answering `rerun` makes the copy count again. The park citation is `finish_parking` (`scheduler.rs:2654`), where parking cancels open attempts. `scheduler.rs:2872` and `:2914` are other paths, so I did not cite them.

2. **`shown_where` changes.** Changed. `shown_where`'s filter now includes `carries_revision` (section "Actions and effects", item 2). `review_copy` and `shown_where` share that predicate. The doc comment update is stated there too. `PathsView.plan` and `plan_files` therefore see the revised copy. The `shown_lane_files` and `ticket_paths` readers are covered by the same change.

3. **The rerun clause.** Changed. The clause no longer says "with the owner's revision". It names the copy and the feedback file, and says the revision was sent, not that the planner applied it (section "Actions and effects", item 5).

4. **Which attempt the clause names.** Changed. The clause is computed by a new `revised_copy_carries(t, a)`, not `rerun_carries`, which stays unchanged. It names the attempt `review_copy` returns, so the `review/1` revised and `review/2` failed case names `review/1`. The subject is the single key of `a.artifacts`, as `start_workflow` writes it (`scheduler.rs:7493`). The clause is empty unless the returned attempt is not `Complete` (section "Actions and effects", item 5). The test "a_rerun_after_a_revise_starts_from_the_revised_copy" checks the feedback path in the question.

5. **Docs.** Changed. "Docs" now lists `docs/design.md:2533`–`2537` (`plan_files` and the open-review copy) and `docs/dispatch-agent-guide.md:208`–`216` (the `show` labels). The agent guide change describes the new `(reviewed copy, round N, review failed)` note and the `(prior revision, round N)` entries.

6. **Contract handling.** Changed. "The record" section now says `PathsView` and `LaneFile` are in no record. I checked: no reference in `ticket.rs` or `store.rs`. They cross the port (`src/ports/dispatch.rs:11`) and `show --json`, so `RECORD_VERSION` stays. "Contract additions" lists both new fields, and the commit message stays a title line as the request asked. The additions are the fields `LaneFile.prior` and `LaneFile.failed`, both serde default false.

7. **`plan_files` literal.** Changed. "UI" now says the `serve.rs:541`–`548` literal gains `prior: false` and `failed`, since it has no `..LaneFile::default()`.

8. **Lane for `prior` entries.** Changed. `lane_files_by` returns only the newest attempt per lane, so it cannot list superseded revisions. "UI" now adds `prior_lane_files`, a new walk beside `lane_files_by` that keeps every superseded revised attempt, with its lane from `a.context` when the stage runs per lane. The `PathsView.plan` and `LaneFile` doc comments are listed for update (`dispatch-control/src/lib.rs:546`–`550` and `:586`–`600`).

9. **The label applies to every finished review.** Changed. The rule is restricted. The new `(reviewed copy, round N, review failed)` note prints only for a shown copy whose attempt failed. Prior entries print only for superseded revised attempts. A finished, non-failed copy prints exactly as today, and "UI" says so. The label logic is in one function `lane_file_note`, so the test can cover it.

10. **Unit test.** Changed. "Tests" now requires a unit test in the `#[cfg(test)]` module at `dispatch/src/main.rs:1898`, covering the three labels and the case that returns none.
