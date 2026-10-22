# Response to feedback 1

1. **Changed: "The UI".** `prior_lane_files` no longer hangs off a shown
   Workflow row. It walks the ticket's Workflow attempts by (stage,
   context) and asks the shared helper, so the prior row appears while
   `plan/1` is the shown row and the `rerun` question is pending or its
   answer is unacted, and stays while `review/2` runs. "The tests" adds
   `ticket_paths` cases for each of those windows and for `review/2`
   Complete, where the row goes away.

2. **Changed: "The actions and effects" and "The UI".** One helper,
   `revised_source` (with `revised_source_gone`), holds the rule: a
   Failed or Cancelled Workflow attempt of the same stage and context,
   newer than the chosen Complete attempt, with a revision, whose file
   exists. `start_workflow`, the rerun sentence and `show` all call it.
   The "same rule" claim is now true. Tests cover a revised attempt older
   than a later Complete one and a missing file, for both `show` and the
   scheduler.

3. **Changed: "The actions and effects".** The subject name is read from
   the workflow attempt's `artifacts`, which has exactly one key
   (`start_workflow` builds it that way), so no pipeline is needed in
   `rerun_ask` or at the resume call (8294). An attempt without exactly
   one key gets the empty sentence, as today. The sentence is also empty
   when there is no Complete source or it has no session, since the
   question cannot return an error and `start_workflow` parks with the
   reason. A `rerun_ask` test covers both.

4. **Changed: "The actions and effects".** The sentence now says "If
   `<path>` is still there, the next attempt reviews it", so it states
   what the rerun does if nothing changed. `start_workflow` logs a
   `log::warn!` naming the missing path and the original it fell back
   to; the plan notes the fallback was silent before. A test covers it.

5. **Changed: "Where this departs from the issue", "The actions and
   effects", "The tests", and the changelog and docs text.** I did not
   widen `WorkflowStart` to pass the round file; that would change the
   contract for a partial case. The plan instead says the gap plainly:
   the question names the round file, the `show` line says "owner
   revised" not "applied", and the changelog and the "A review is a
   copy" sentence say the rerun starts from the copy the revise was sent
   to, with the note in the round file if the planner had not applied it.
   The overstated "starts from the owner's revised plan" is gone.

6. **Changed: "The tests".** Added
   `a_rerun_after_a_lost_send_still_starts_from_the_revised_copy`, which
   pins the documented behaviour: the revision is kept on a lost send,
   so the rerun starts from that copy and the question names the round
   file. "Where this departs" now names the lost-send case as part of
   the same gap.

7. **Changed: "The UI" and "Contract additions".** `dispatch-control`
   has no protocol or version constant (checked in `lib.rs`), so
   `show --json` needs no bump; the plan says so. The struct literals in
   serve.rs near 1469 and 1540 do not use `..LaneFile::default()` and get
   `prior: None`; the others compile unchanged.
