# Plan: #187, a plan review rerun keeps the owner's revision

References are to this worktree. "Facts" is `facts.md` for #187; "Settled" lines are cited by their claim.

## Where this departs from the issue

- The issue says the `revise` note "should survive a runner failure the same way an answered decision does". It already does: `revise()` writes it to `review/<n>/joined/plan.feedback-<round>.md` before sending and deletes it only if Switchboard refuses (`dispatch/src/scheduler.rs:7616-7642`; Facts, Settled: the note survives on disk). The plan adds no copy of the note to the record.
- The issue frames the loss as a rerun-after-failure bug. The same `review_subject` rule (`scheduler.rs:10534-10548`, called at `:7465`) loses the revision on park then resume, where a cancelled review attempt is rerun through `rerun_ask`'s `Cancelled` arm (`:9363-9368`; Facts, Settled). The fix sits in the shared path, so it covers both.
- The investigation says the record is "schema version 19". That is when `revisions` was added (`docs/dispatch.md:393`). The version today is `RECORD_VERSION = 28` (`dispatch/src/store.rs:26`). `SCHEMA_VERSION = 16` at `store.rs:12` is the Switchboard app store's and is not touched.
- The issue's "at least say in the rerun question and in `show` which copy it will review" is done in full. The question text and `show` both name the copy.

## The record

One additive field on `Attempt` (`dispatch/src/ticket.rs`, beside `revisions` at `:350`): `seeded_from: Option<u32>`, `#[serde(default)]`. It is the number of the earlier attempt of the same stage and context whose copy this attempt's reviewed copy was copied from. `None` means copied from the newest complete attempt, as today.

Why a field and not a derivation from the `rerun` decision: a later reader (`show`, a third attempt) needs to know what the copy holds, and a rerun answer can be withdrawn or re-asked. The record should say what was reviewed.

Bump `RECORD_VERSION` to the value in `store.rs` at implementation time, plus one. Add the `migrate` step, which transforms nothing (the neighbouring 18-to-19 step is the pattern, `store.rs:~555`). The comment says a build that would drop the field on its next write must refuse the record, or a third attempt would restart from the original. Add the version line to `docs/dispatch.md` after the existing version list (`:385-400`).

One helper, `Attempt::holds_revision()`: `!revisions.is_empty() || seeded_from.is_some()`.
- It is not proof the reviser finished. Facts, Settled: a non-empty `revisions` means the revise was sent or its fate unknown (`NO_ANSWER` keeps it, `scheduler.rs:11285-11313`).
- `seeded_from` is set only when a revision was carried, so a chain of failures (review/2 fails, then review/3) keeps carrying.
- Facts, For the owner: the plan must not rely on `revisions` alone. It does not. The helper only decides whether to offer a choice and to default to the carried copy. The owner's answer decides, and the question shows the path.

## The signals

None new. Runner failure (`review run gone`, `scheduler.rs:7757-7763`) stays terminal for the attempt, and the plan does not try to re-attach the run (Facts, Unresolved: why the app answers `no such run`).

## The actions and effects

No new `Effect`, `Body` or port method. All changes are in `dispatch/src/scheduler.rs`.

1. **Rerun options.** A new `fn rerun_options` returns `["rerun", "original", "park"]` for a failed or cancelled `Workflow` attempt where `holds_revision()` is true and its copy exists on disk, else the current set.
   - `rerun_ask` (`:9335-9379`) calls it for both its `Failed` and `Cancelled` arms. This covers park then resume.
   - `fail_attempt` (`:7157`) passes it instead of the fixed `["rerun", "park"]`.
   - `rerun` is the default and carries the revised copy forward. `original` is today's behaviour. This is the design that works whichever way the owner's answer on "is the revised plan the one I approved" goes (Facts, For the owner): `original` is always one answer away.
2. **Question text.** `rerun_carries` (`:9382-9392`) gets a `Workflow` arm for the `holds_revision()` case: `` `rerun` reviews <path of the failed attempt's plan.md>, the copy you revised, and `original` reviews <plan stage's path> again. If the review died while the planner was editing, the copy may be incomplete. ``
   - This is the visibility Facts, Unresolved (mid-edit kill) asks for. Nothing is copied silently, and the plan makes no claim about completeness either way.
   - `ask_rerun` and `fail_attempt` both build their text through this function, so the first question and a re-ask after a park read the same.
3. **Answer handling.**
   - `Decision::acted_rerun` (`ticket.rs:930`) accepts the answer `original` as well as `rerun`, so `may_rerun` (`scheduler.rs:10665`) launches on either.
   - The two matches on `("rerun", "rerun")` (`:3468`, `:3524`) become `("rerun", "rerun" | "original")`. Only the first has a note; `original` carries none.
   - `ANSWERS` (`pipeline.rs:~114`) gets `("original", "`rerun`")`, as `keep` and `check` have.
   - The authorised-unlaunched check at `:10674` goes through `acted_rerun` and needs no change.
4. **`start_workflow`** (`:7436`). Before the copy at `:7458-7471`:
   - Let `prev = latest_attempt(t, &stage.name, ctx)`. If it is a failed or cancelled `Workflow` attempt, `holds_revision()` is true, its answer for this attempt is not `original`, and its `plan.md` exists, use that file as `source_path` and set `seeded_from = Some(prev.n)` on the new attempt after the push at `:7476`.
   - `source_session` still comes from `review_subject`, unchanged. The app clones the session that wrote `plan/1`, and it cannot see the owner's note (Facts, Settled: only the file carries the revision). The carried file holds the revised text, so nothing is handed to the clone. If a reviewer turns out to need the note, that is the follow-up below, because `WorkflowStart` has no field for it and this commit adds none.
   - If the carried file is missing, log a warning and fall back to the original. The question only offers the choice when the file existed at ask time, so this is a race, not a path.
5. **Readers left alone.** `review_subject` (the session and the first copy), `input_where` (`ticket.rs:1452`), `shown_where` (`:1476`) and the lane readers keep their `Complete`-only rule. A failed review's copy must never feed a later stage's prompt, and the Plan tab contract is that agents are handed complete files only (`docs/design.md:~2158`, quoted in the investigation). `report.rs` `plan_review` (`:355-380`) counts rounds per attempt from each copy's feedback files, so it is unaffected. The new attempt starts at round 1 with no feedback files, and the owner's round stays with review/1.

## The UI

`show` and its wire types only. `dispatch-control` additions (additive, struct-level `#[serde(default)]`, `lib.rs:584`):
- `LaneFile.seeded_from: Option<u32>`.
- `PathsView.plan_earlier: Vec<EarlierCopy>` with `EarlierCopy { stage, n, path, state, owner_revised }`.

`ticket_paths` (`serve.rs:~539`) fills them. `plan_earlier` lists failed or cancelled workflow attempts that `holds_revision()` and are not the shown copy. `print_lane_files` (`main.rs:896-907`) gets:

```
  plan: .../review/2/joined/plan.md
    (reviewed copy, review open; started from review/1's revised copy)
  plan, earlier: .../review/1/joined/plan.md (review/1 failed, owner-revised)
```

When `seeded_from` is `None` and a plan review is open, the line says `started from stage plan`. Between the failure and the answer, `show` still names `plan/1` as the shown plan, because `shown_where` is unchanged. The `plan, earlier` line and the question are what flag the revision there.

The Dispatch page's plan label (`src/ui/ticket.rs:~425`) reads the same `PathsView`, so it receives the fields. New wording there is out of this commit.

## The tests

Dispatch scheduler tests, beside the existing review tests near `scheduler.rs:12836`, with the fake app and `Clock::at`:
- Revised review/1 fails, `rerun` is answered: the review/2 copy has review/1's bytes, `seeded_from == Some(1)`, and `WorkflowStart.plan` is the review/2 path with plan/1's session as `source`.
- Same, answered `original`: the copy has plan/1's bytes and `seeded_from == None`.
- Review/1 failed unrevised: today's behaviour, options `["rerun", "park"]`.
- Chain: review/2 (seeded) fails, `rerun` gives review/3 the review/2 copy; `original` on review/2 then makes review/3 unseeded.
- Park then resume of a revised open review: the cancelled attempt's question offers `original` and the rerun carries the copy.
- The question text names both paths and the incompleteness caveat. A missing carried file falls back to the original.
- `Decision::acted_rerun` is true for `original` (`ticket.rs` tests near `:1819`).
- Store: a record at the previous version loads with `seeded_from == None`, migrates to the new `RECORD_VERSION` (pattern at `store.rs:971`), and a newer one is refused.
- `serve.rs` test: `ticket_paths` fills `seeded_from` and `plan_earlier`, with a snapshot of the `show` lines.
- No UI test: `tests/ui.rs` is untouched because the app's pages gain no wording.

## Docs and housekeeping

- `docs/dispatch.md`: the new version line, and the plan review paragraph says a rerun carries the owner-revised copy and offers `original`.
- `README.md`: no new file, so no Layout change.
- `docs/design.md`: no status entry for a Dispatch item of this size.
- `CHANGELOG.md`, under Unreleased: "Dispatch: a plan review rerun after a failed or parked attempt starts from the owner's revised copy, and the rerun question and `show` say which copy is reviewed; answer `original` to start from the plan again."
- No client names anywhere. Test fixtures say "a pipeline".
- Run `cargo test --locked --workspace`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, `cargo fmt --all`, and the Linux cross-check for the dispatch crate. One commit. The app runs at every step: the field defaults, and the old behaviour is the `None` path.

## Follow-ups for the owner (not in this plan)

- Cloning the app's rerun planner from review/1's planner session instead of plan/1's would carry the note by transcript. That changes the clone-source rule in `src/core/workflow.rs:289`, so it is a separate decision.
- Why the app loses the run after a runner restart (Facts, Unresolved). If it is recoverable, re-attaching beats rerunning.
- `rerun --note` on a plan review is dropped (`rerun_note`, `scheduler.rs:8996-8999`). It stays on the decision record only.
- Confirm against the issue's files that `review/1/joined/plan.md` differs from `review/2/joined/plan.md` and that the feedback file holds the quoted note (Facts, For the owner). The plan's behaviour does not depend on the answer.

## Commit message

Dispatch: a plan review rerun starts from the owner's revised copy and says which copy it reviews

## Contract additions

- `Attempt.seeded_from: Option<u32>` (`dispatch/src/ticket.rs`), serde default; `Attempt::holds_revision()`.
- `RECORD_VERSION` bumped from the value in `dispatch/src/store.rs` at implementation time, plus one, with a no-transform `migrate` step.
- New `rerun` answer `original`, for a workflow attempt that holds a revision; `Decision::acted_rerun` accepts it, and `pipeline.rs` `ANSWERS` lists it.
- `dispatch-control`: `LaneFile.seeded_from: Option<u32>`; `PathsView.plan_earlier: Vec<EarlierCopy>`; new `EarlierCopy { stage, n, path, state, owner_revised }`.
