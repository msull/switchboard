# A plan review's rerun starts from its newest copy (#187)

## Where this departs from the issue

- **"Starts from the plan stage's original output" is right, but the cause is not the `review run gone` path.** `start_workflow` (`dispatch/src/scheduler.rs:7436`) takes the reviewed artifact from `review_subject` (`scheduler.rs:10534`), which walks the attempts newest first and keeps only those with `state == Complete`. A failed or cancelled review attempt is skipped whatever it holds, so the plan stage's attempt is the first match. Every way a review attempt ends short of `finalize` has the same effect: `review run gone` (`scheduler.rs:7761`), `review: <reason>` from a paused-and-failed run (`scheduler.rs:7823`), a park and resume that cancels the attempt, and `could not start the review` (`scheduler.rs:7525`). The fix is in the input choice, not in the `no such run` arm.
- **"The `revise` note should survive a runner failure like an answered decision" already holds.** The answered decision keeps its `note`, the attempt keeps `revisions` (`ticket.rs:350`), and the round file `plan.feedback-<n>.md` stays beside attempt 1's copy. None of these is lost. What the rerun loses is the *effect* of the note, which sits in attempt 1's edited copy. Starting from that copy restores it, so the plan adds no store for the note. The one case it does not cover is a `revise` the planner never answered before the run died. For that the rerun question and the new attempt's `finalize` question say so, because a rerun cannot re-send a paid objection on its own.
- **"`show`'s `files:` block names only the attempt-2 copy" is correct,** and it is by design: `Ticket::shown_where` (`ticket.rs:1476`) lists complete attempts and open workflow attempts only. The plan adds the lineage to `show` rather than changing what `plan:` names.
- **"Say in the rerun question which copy it will review"** is not true today: `rerun_carries` (`scheduler.rs:9382`) returns an empty string for a workflow attempt. The plan adds that sentence.

## The record

An attempt gains `reviewed_from: Option<(String, u32)>`: the stage and number of the earlier attempt whose copy a plan review attempt started from. It is `None` for a review of the plan stage's own output, for every other attempt kind, and for records written before this field. It is `#[serde(default)]` and set in `start_workflow` only. `new_attempt` (`scheduler.rs:10277`) initialises it to `None`. It is not read to decide anything. `show`, the question text and the ticket page read it, so a reader can tell which copy was reviewed without guessing from file order.

Schema: `RECORD_VERSION` (`dispatch/src/store.rs:26`, 28 today) becomes the value at implementation time plus one, because a ticket landing in parallel may take the next number first. Add the matching "N to N+1" step in `migrate` (`store.rs:452`) with the comment pattern of its neighbours: nothing is transformed, and a build that would drop the field on its next write must refuse the record, or `show` would lose which copy a rerun reviewed. Switchboard's `SCHEMA_VERSION` (`src/core/model.rs:12`, 16 today) and `VIEWS_SCHEMA_VERSION` (`model.rs:612`, 7 today) do not change. Nothing in the app's workspace or views records is touched.

## The input rule

Replace `review_subject` with `review_input(t, stage, ctx, subject) -> Result<ReviewInput, String>`, a pure function of the ticket apart from one `Path::exists`. `ReviewInput` is `{ path, session, from }`.

- **`path`**: walk `t.attempts` newest first (push order) and take the first attempt that has `subject` in `artifacts` and is either
  - `Complete`, whatever its stage (today's rule), or
  - an attempt of kind `Workflow` in this `stage` and `ctx`, in any state but `Complete`, whose file exists.

  Push order is the right order. If the plan stage runs again after a failed review (a send-back), its attempt is newer and wins, so a stale copy never shadows a fresh plan. A failed attempt whose file is gone is skipped with a `log::warn!`, and the walk falls through to the next candidate. `from` is `Some((stage, n))` when the winner is a review attempt.
- **`session`**: unchanged. It is the newest `Complete` attempt holding `subject` that has a session, because `WorkflowStart.source` (`control/src/lib.rs:215`) names the session the planner clone is made from. That is still the plan stage's session after a failed review.
- The existing "nothing wrote {subject}" and "was not written by a session" errors stay, and still park the stage.

`start_workflow` copies `path` into the new attempt's directory (`scheduler.rs:7471`), sets `reviewed_from`, and logs `ticket <id> <stage>/<ctx> attempt <n> reviews <from>'s copy`. Nothing else in the launch changes. The previous copy is left untouched, so the owner can diff the two. The reviewer processes of the replaced attempt are already retired before `may_rerun` allows the launch (`retire_replaced`, `scheduler.rs:3578`), so the copy is not being written while it is read.

The two questions use the same function, so what they say is what the launch does:

- `rerun_ask` (`scheduler.rs:9335`) appends, for a `Workflow` attempt whose `review_input` is a prior review copy, `A rerun reviews <path>, this attempt's copy with the planner's edits and the owner's revise of round <n>; the plan stage's output is at <path>.` The revise clause appears only when `revisions` is non-empty.
- If the last revision's `response_file` (`report.rs:202`) does not exist, add `The owner's round <n> note (<feedback path>) was not answered; answer revise at finalize to send it again.`
- `review_done`'s `finalize` question (`scheduler.rs:7695`) appends `It started from <stage>/<n>'s copy.` when `reviewed_from` is set.

## The signals

No new event kind and no new log line the readers parse. The `log::info!` above is for the log only.

## The actions and effects

No new action or effect. The behaviour change is which path `Body::WorkflowStart { plan, .. }` carries. The wire body does not change.

## The UI

`dispatch-control` (`dispatch-control/src/lib.rs`) gains `PlanCopy { stage, n, state, path, rounds, revised, from, shown }` and `PathsView.plan_copies: Vec<PlanCopy>` (`serde(default)`, after `plan_files`, `lib.rs:546`). `revised` is the list of owner-revision rounds. `from` is the `reviewed_from` label, such as `review/1`. `shown` marks the copy `plan` names. `serve::ticket_paths` (`dispatch/src/serve.rs:535`) fills it, oldest first, from every attempt that holds `plan` and is a complete attempt or a workflow attempt. It is empty when there is only one such attempt, so a ticket with one copy looks as it does now.

- **`dispatch show`**, in the `files:` block after `plan` (`dispatch/src/main.rs:981`):

  ```
    plan copies:
      plan/1 complete: <path>
      review/1 failed, revised in round 1: <path>
      review/2 running, started from review/1: <path> (shown)
  ```

  This is a new `print_plan_copies` beside `print_lane_files` (`main.rs:896`). Attempts and `plan:` print as they do today.
- **The ticket page's Plan tab** (`src/ui/ticket.rs:426`): `plan_label` gains a second meta line when the shown copy has a `from`: `Started from review attempt 1's copy (revised in round 1); the plan stage's output is untouched.` The tab shows nothing new when `plan_copies` is empty, which includes any runner that predates the field.

## The tests

- `dispatch/tests/first_slice.rs`, using `at_finalize`, `revise` (`first_slice.rs:3416`) and the `runs.clear()` pattern of `a_run_switchboard_no_longer_has_fails_the_attempt` (`first_slice.rs:3810`):
  - `a_rerun_after_a_revise_reviews_the_revised_copy`: revise, overwrite the attempt 1 copy as the planner would, clear the runs, step. The `rerun` question names attempt 1's copy. Answer `rerun`, step. The attempt 2 copy has the revised bytes, `reviewed_from == Some(("review", 1))`, the `WorkflowStart` plan is attempt 2's copy, and its `source` is still the plan stage's session.
  - `a_rerun_after_a_failure_with_no_revise_still_reviews_the_plan`: the copy is a byte-for-byte match of the plan, `reviewed_from` is set, and no revise clause is in the question.
  - `a_revise_the_planner_never_answered_is_named_in_both_questions`: no response file for the last revised round. The rerun question and the new `finalize` question both say so.
  - `a_rerun_after_a_park_and_resume_starts_from_the_cancelled_copy`: the cancelled attempt takes the same path.
- `dispatch/src/scheduler.rs` unit tests for `review_input`: a failed review copy beats the plan; a newer plan attempt beats an older failed copy; a failed copy whose file is gone falls back to the plan; a `Complete` review attempt is not offered to its own stage's rerun; another stage's failed copy and another context's are ignored.
- `dispatch/src/store.rs`: a record at the previous version migrates with `reviewed_from` absent and round-trips at the new one; a record newer than `RECORD_VERSION` is refused (the pattern of `a_version_eighteen_record_migrates_with_no_revisions`, `store.rs:1278`).
- `dispatch/src/serve.rs`: `ticket_paths` lists the three copies in order with `shown` on the last, and an empty list for a ticket with one (beside the tests at `serve.rs:1468`).
- `dispatch-control/src/lib.rs`: a `PathsView` JSON with no `plan_copies` still parses (beside `lib.rs:1008`).
- `tests/ui.rs`, beside the plan-label test at `tests/ui.rs:5859`: the second meta line shows with a `from` and not without.
- Docs: `docs/dispatch.md` "A review is a copy" (line 550) says a rerun starts from the newest copy of its own stage; a row in the failure table next to the `workflow` query row (line 3078) names the four tests. `docs/design.md` "Dispatch ticket page (2026-10-04)" (line 2135) gets a sentence on the second meta line. `README.md` needs no new Layout line, since no file is added; check the `dispatch/` and `src/ui/ticket.rs` lines still describe the files. `CHANGELOG.md` gets a line under Unreleased.

Commit message: `Dispatch: a plan review rerun starts from the newest copy of its own stage, and show lists the copies`

Contract additions:
- `ticket::Attempt.reviewed_from: Option<(String, u32)>` (`serde(default)`), with `store::RECORD_VERSION` raised by one from the value in `store.rs` at implementation time and a matching step in `store::migrate`.
- `dispatch_control::PlanCopy` and `PathsView.plan_copies: Vec<PlanCopy>` (`serde(default)`).
- `scheduler::review_input` and `ReviewInput`, replacing the private `review_subject`.
- No new `AppAction`, `Effect`, port method or `Body` variant; `SCHEMA_VERSION` and `VIEWS_SCHEMA_VERSION` do not change.
