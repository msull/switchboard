# Plan: #187, a plan review rerun keeps the owner's revision

References are to this worktree. "Facts" is `facts.md` for #187; "Settled" lines are cited by their claim.

## Where this departs from the issue

- The issue says the `revise` note "should survive a runner failure the same way an answered decision does". It already does: `revise()` writes it to `review/<n>/joined/plan.feedback-<round>.md` before sending and deletes it only if Switchboard refuses (`dispatch/src/scheduler.rs:7616-7642`; Facts, Settled: the note survives on disk). The plan adds no copy of the note to the record.
- The issue frames the loss as a rerun-after-failure bug. The same `review_subject` rule (`scheduler.rs:10534-10548`, called at `:7465`) loses the revision on park then resume, where a cancelled review attempt is rerun through `rerun_ask`'s `Cancelled` arm (`:9363-9368`; Facts, Settled). The fix sits in the shared path, so it covers both.

## The record

One additive field on `Attempt` (`dispatch/src/ticket.rs`, beside `revisions` at `:350`): `seeded_from: Option<u32>`, `#[serde(default)]`. It is the number of the earlier attempt of the same stage and context whose copy this attempt's reviewed copy was copied from. `None` means copied from the newest complete attempt, as today. A later reader (`show`, a third attempt) needs to know what the copy holds, and a rerun answer can be withdrawn or re-asked, so the record says what was reviewed.

Bump `RECORD_VERSION` to the value in `store.rs` at implementation time, plus one. Add a `migrate` step with no transform, the same pattern as 18 to 19. Add the version line to `docs/dispatch.md` after the existing version list (`:385-400`).

`Attempt::holds_revision()`: `!revisions.is_empty() || seeded_from.is_some()`. It only decides whether to offer `original` and whether to default to the carried copy. `seeded_from` is set only when a revision was carried, so a chain of failures keeps carrying.

## The signals

None new.

## The actions and effects

No new `Effect`, `Body` or port method. Changes are in `dispatch/src/scheduler.rs`, plus one answer in `pipeline.rs`, one seed line in `supervisor.rs` and `Decision::acted_rerun` in `ticket.rs`.

1. **Rerun options.** A new `fn rerun_options` returns `["rerun", "original", "park"]` for a failed or cancelled `Workflow` attempt where `holds_revision()` is true, its copy exists on disk and the currency check in item 4 passes, else the current set. `rerun_ask` (`:9335-9379`) calls it for both its `Failed` and `Cancelled` arms (this covers park then resume); `fail_attempt` (`:7157`) passes it instead of the fixed `["rerun", "park"]`. `rerun` is the default and carries that copy forward; `original` is today's behaviour.
2. **Question text.** `rerun_carries` (`:9382-9392`) gets a `Workflow` arm for the `holds_revision()` case: `` `rerun` reviews <path of the failed attempt's plan.md>, the copy a revise was sent to, and `original` reviews <plan stage's path> again. If the review died while the planner was editing, the copy may be incomplete. `` When `seeded_from` is set and `revisions` is empty, it says "the copy this attempt started from". `ask_rerun` and `fail_attempt` both build their text through it.
3. **Answer handling.**
   - `Decision::acted_rerun` (`ticket.rs:930`) accepts `original` as well as `rerun`, so `may_rerun` (`scheduler.rs:10665`) launches on either.
   - The two matches on `("rerun", "rerun")` (`:3468`, `:3524`) become `("rerun", "rerun" | "original")`. Only the first has a note; `original` carries none.
   - `ANSWERS` (`pipeline.rs:111`) gets `("original", "`rerun`")`, as `keep` and `check` have.
   - `decision_words("rerun")` (`supervisor.rs:69-73`) gains: "`original` reviews the plan stage's copy again, when the question offers it".
4. **`start_workflow`** (`:7436`). Before the copy at `:7458-7471`, let `prev = latest_attempt(t, &stage.name, ctx)`. If it is a failed or cancelled `Workflow` attempt, `holds_revision()` is true, the answer is not `original`, its `plan.md` exists, **and the plan it was copied from is still current**, use that file as `source_path` and set `seeded_from = Some(prev.n)` on the new attempt after the push at `:7476`. A missing carried file logs a warning and falls back to the original.
   - Currency, `subject_unchanged_since(t, &subject, prev)`: no `Complete` attempt that wrote `subject` sits after `prev` in `t.attempts` (chronological; `review_subject` walks it in reverse on that assumption). A plan redone after review/1 started puts a newer `Complete` plan attempt after it, and the check fails.
   - When it fails, copy from `review_subject` as today, `seeded_from` stays `None`, and log a warning naming the newer plan attempt. The `rerun` answer is honoured; only the carry is withheld. `rerun_options` runs the same check, so a plan redone before the question offers no `original`; `start_workflow` checks again because a plan can be redone after it.
   - `source_session` still comes from `review_subject`, unchanged.
5. Don't change the `Complete`-only readers (`review_subject`, `input_where`, `shown_where`): a failed review's copy must never feed a later stage's prompt.

## The UI

`show` only. `dispatch-control` gains `LaneFile.seeded_from: Option<u32>` (struct-level `#[serde(default)]`, `lib.rs:584`), filled in `ticket_paths` (`serve.rs:~539`). `print_lane_files` (`main.rs:896-907`) prints:

```
  plan: .../review/2/joined/plan.md
    (reviewed copy, review open; started from review/1's copy)
```

When `seeded_from` is `None` and a plan review is open, the line says `started from stage plan`.

## The tests

Dispatch scheduler tests, beside the existing review tests near `scheduler.rs:12836`, with the fake app and `Clock::at`:
- Revised review/1 fails, `rerun` is answered: the review/2 copy has review/1's bytes, `seeded_from == Some(1)`, `WorkflowStart.plan` is the review/2 path with plan/1's session as `source`, and the question text names both paths and the incompleteness caveat. Answered `original`: the copy has plan/1's bytes and `seeded_from == None`.
- Plan redone after the failure, then `rerun`: the review/2 copy has plan/2's bytes and `seeded_from == None`. Plan redone before the question: options are `["rerun", "park"]`.
- Review/1 failed unrevised: options `["rerun", "park"]`.
- Chain: review/2 (seeded) fails, `rerun` gives review/3 the review/2 copy; `original` on review/2 makes review/3 unseeded.
- Park then resume of a revised open review: the cancelled attempt's question offers `original` and the rerun carries the copy. A missing carried file falls back to the original.
- Supervisor seed: the `rerun` line of `decision_words` mentions `original`; a pipeline whose `decides` lists `original` is refused with the `ANSWERS` message (tests near `pipeline.rs:2483`).
- `Decision::acted_rerun` is true for `original` (`ticket.rs` tests near `:1819`).
- Store: a record at the previous version loads with `seeded_from == None` and migrates (pattern at `store.rs:971`).
- `serve.rs`: `ticket_paths` fills `seeded_from`, and the `show` line matches.

## Docs and housekeeping

- `docs/dispatch.md`: the version line (see The record).
- `CHANGELOG.md`, under Unreleased: "Dispatch: a plan review rerun after a failed or parked attempt starts from the copy the revise was sent to, and the rerun question and `show` say which copy is reviewed; answer `original` to start from the plan again."

## Follow-ups for the owner

- `rerun --note` on a plan review is dropped (`rerun_note`, `scheduler.rs:8996-8999`); it stays on the decision record only.
- If a reviewer needs the note itself, cloning from review/1's planner session changes the clone-source rule (`src/core/workflow.rs:289`) and `WorkflowStart` has no field for it; a separate decision.

## Commit message

Dispatch: a plan review rerun starts from the owner's revised copy and says which copy it reviews

## Contract additions

- `Attempt.seeded_from: Option<u32>` (serde default); `Attempt::holds_revision()`.
- `RECORD_VERSION`: from the value in `store.rs` at implementation time, plus one; no-transform `migrate` step.
- `rerun` answer `original` (`acted_rerun`, `ANSWERS`, `decision_words("rerun")`).
- `dispatch-control`: `LaneFile.seeded_from: Option<u32>`.
