# Response to round 2

I made every cut, including the large one. One sentence in the plan's "Where this departs from the issue" section still points to the `Cancelled` rationale; the other repetitions are gone.

**Earlier points**
1. Closed, nothing to change.

**Where this departs from the issue**
- `Cancelled` rationale: the line list now sits only under Path in "The actions and effects". The departure bullet keeps a one-clause pointer.
- "Step 2 of the repro" bullet: cut.
- `show` bullet: kept once, reworded to say the plan states the copy in the rerun question and leaves `show` alone. "The UI" is gone.

**The record**
- `started_from` is cut. I did not defer it as a follow-up in the plan body. It appears as one Follow-ups bullet, since it is a separate decision.
  - The field, the `RECORD_VERSION` bump and the `migrate` step are gone.
  - `LaneFile.started_from`, the `ticket_paths` change and the `print_lane_files` line are gone.
  - The store, serve and control tests are gone.
  - `Subject` and `from` are gone too, so `review_subject` keeps its `(PathBuf, String)` return.
- The section is now one line saying there is no record change.

**The signals**
- Section cut.

**The actions and effects**
- The opening "No new AppAction…" paragraph: cut.
- The tuple-to-struct paragraph: cut with `started_from`.
- "Chained failures work" bullet: cut.
- Step 3 ("The default does not depend on the unresolved facts"): cut. I kept one clause at the end of step 2 saying the owner can open the copy and `park`, because that is the owner's way out when U1 bites.
- The `Cancelled` widening sentence in Path: cut.

**The UI**
- The whole section is cut.

**The tests**
- Optional `NO_SUCH_RUN` test and its fallback: cut.
- Store, serve and control tests: cut.
- Closing `fmt`/`clippy`/`test` line: cut.
- "Read the fake first": cut.
- The `started_from` assertions are removed from the gate case.

**Docs and changelog**
- README bullet and the app/`SWITCHBOARD_SCRIPT` bullet: cut.
- The `Ticket::input` sentence: cut.
- The design.md status line no longer mentions the record version or `started_from`.
- The changelog line no longer says `show` reports the copy.

**Follow-ups**
- Plan tab bullet, planner-clone bullet and O1: cut.
- Added one bullet for `started_from`, as the follow-up that replaces the cut field.

**Commit**
- The contract additions list is reduced to one line saying there are none.
