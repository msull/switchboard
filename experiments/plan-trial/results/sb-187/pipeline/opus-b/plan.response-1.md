# Response to round 1

1. **`docs/dispatch.md` left out.** Changed: Docs. A new
   `docs/dispatch.md` bullet covers three places. "A review is a copy"
   (550-557) gets the new rule: a rerun copies the failed attempt's copy
   as the run left it, a "start over" note copies the reviewed artifact
   again, and `carried_from` records where the copy came from. The port
   paragraph (2830-2836) names the tagged earlier rounds and the plan
   file's `from`. The first-slice table gets two rows after the `revise
   --note` row at 2975, one for each of the two new tests, worded like
   the code review rows at 3058 and 3060.

2. **The note is removed before the launch can stop short.** Changed:
   Actions and effects, `start_workflow` steps 1 and 3. Step 1 now reads
   the note with `t.rework.get`, citing the parks at 7449-7467, the
   `Err` returns at 7468-7472, and the rule at `scheduler.rs:6279-6280`
   and 6370. Step 3 removes the note when the attempt is pushed (7491),
   before `save_ticket`.

3. **The Plan tab still loses the owner's round.** Changed: The UI,
   The tests, Docs, Contract additions, and the second bullet of "Where
   this departs from the issue". `PlanRoundView` gains `attempt:
   Option<u32>`. `ticket_paths` walks `carried_from` back and lists the
   earlier attempts' rounds first, each tagged with its attempt.
   `round_file` (`serve.rs:573`) falls back only to the current
   attempt's rounds. `show` prints `(owner, review/{m})`. The Plan tab
   (`src/ui/ticket.rs:454-470`) titles those rounds "Attempt {m} · Round
   {n}" and keys each fold by attempt. The finalize card's count
   (`ui/ticket.rs:154`) counts only the current attempt's rounds. New
   tests: `plan_rounds_list_an_earlier_attempts_rounds_first` in
   `serve.rs` and `the_plan_tab_tags_an_earlier_attempts_rounds` in
   `tests/ui.rs`. The design.md Plan tab bullet (2160-2165) is updated.

4. **A copy left incomplete by a dead planner.** Changed: The signals
   (a new paragraph), the `rerun_carries` wording in Actions and
   effects, and the known-gaps line in Docs. The plan now says the copy
   is taken as the failed run left it. Dispatch cannot tell a partial
   in-place edit from a finished one, so it does not try. The rerun
   question now says "this attempt's copy as the failed run left it",
   and "start over" is named as the remedy both there and in the known
   gaps.

5. **`review_subject` returns no attempt.** Changed: The signals and
   `rerun_carries` in Actions and effects. The walk is split out as
   `subject_writer(t, subject) -> Option<&Attempt>`, with the filter at
   10539-10544 unchanged. `review_subject` becomes a wrapper over it
   that keeps its signature and error texts. `rerun_carries` takes
   `{writer}/{n}` from `subject_writer`, and drops the "start over"
   clause when it returns `None`.
