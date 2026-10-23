# Facts for issue #187

Nothing was run or built. Tree claims are from reading this worktree (HEAD 7fe8949).

## Settled

Tree questions from the investigation:

- The app's `WorkflowStart` takes any absolute `plan` path; it does not need a file the attempt copied itself. The only check on the path is `is_absolute` (`src/core/workflow.rs:289-325`, wire decode `src/core/control.rs:1384-1394`). The planner is cloned from `source`, which must be a Claude Code session with a transcript. So the plan may pass review/1's revised copy as `plan` while `source` stays the plan attempt's session.
- `clean_up` removes only the recorded round files (`feedback` and `response` of each round), never the reviewed copy (`src/core/workflow.rs:859-870`, `src/core/model.rs:394-399`). A cleaned-up review/1 still has its `plan.md`.
- `remove_workflow` drops the run from `workspaces[].workflows`, and only when the run is not waiting (`src/core/workflow.rs:1009-1025`). It touches no Dispatch file. Dispatch sends no `workflow.remove` (investigation §4, grep of `dispatch/`).
- A runner restart does not edit workflow attempts. The only workflow mention in `dispatch/src/restart.rs` is the match arm at line 883, which sets `failed_at_checks` to false. Nothing there touches `artifacts` or `revisions`. `recover.rs` has no reference to `Workflow`, `artifacts` or `revisions` (grep).
- `fail_attempt_with` changes `state` and `ended_ms` and leaves `artifacts` untouched (`dispatch/src/scheduler.rs:7195-7230`, per the investigation). A failed review attempt still names its revised copy as `plan`, and `revisions` is still set.
- The cause is `review_subject` (`dispatch/src/scheduler.rs:10534-10548`). It keeps only `AttemptState::Complete` attempts, so the failed review/1 is skipped and the plan stage's output is used. `Ticket::input_where` has the same filter (`dispatch/src/ticket.rs:1452-1463`).
- `show` and the rerun question behave as the investigation's §2 and §1b say: they name no copy and ignore review/1's revision (`dispatch/src/serve.rs:538-594`, `dispatch/src/main.rs:981-998`, `dispatch/src/scheduler.rs:9335-9390`).
- `Revision` is `#[serde]` data on `Attempt` (`dispatch/src/ticket.rs:346-350`). A test pushes one at `ticket.rs:1293`.

Outside the tree:

- Claude Code deletes session transcripts older than `cleanupPeriodDays` (default 30), and a deleted transcript cannot be resumed. Covers Claude Code 2.1.x. Evidence: `spikes/01-claude-resume/README.md:7` and `:278-285` (read from the binary's schema), and https://claudeissues.com/issue/59248-silent-retention-cleanup-deletes-session-transcripts-with-no-warning-opt-in-or-r (a report that agrees). This bears on the plan only if a rerun is days old. The rerun clones from the plan attempt's session, the same one it clones from today.
  - Query: `Claude Code cleanupPeriodDays session transcripts deleted --resume session files retention`

## Unresolved

- **Codex `resume <id> "<prompt>"` when the rollout is missing** (`docs/design.md:1173-1174` lists it as unverified).
  - Query: `OpenAI Codex CLI "codex resume" session id prompt rollout file missing error`.
  - Only third-party blogs came back, and none documents that case.
  - Line to build to: the fix does not need it. A review rerun starts a fresh run through `WorkflowStart`, with a new reviewer, and resumes nothing. Add nothing that resumes a Codex reviewer. If it is later verified, nothing in this plan changes.
- **Why the app answered `no such run`** for the lost run after a runner restart. The tree shows the run was absent from `workspaces[].workflows` at the query. It does not show why. It was not reproduced, and the reproduction needs a live app and the owner's data.
  - Line to build to: the plan must not depend on the cause. Whatever removed the run, a Failed review attempt keeps its `artifacts` and `revisions` on the Dispatch side, so the plan must pick the rerun's input from the ticket record alone.

## For the owner

- Did anything send `workflow.remove` or `Clean up` on the run between the revise and the failure? Whether or not it did, the fix is the same: the rerun reads the failed attempt's record. If the answer is yes, the fix is also unchanged. `clean_up` leaves the reviewed copy, and the run's removal does not touch Dispatch's files.
- On the owner's ticket, does `review/1/joined/plan.md` hold the revision, and is `review/2/joined/plan.md` byte-identical to `plan/1/joined/plan.md`? If yes, this matches the code path above. If no, the cause lies elsewhere and the plan stays the same for the Failed-attempt case. The owner then reports the diff on the issue.
- The issue's other claims about the run's contents are about the owner's ticket and cannot be checked here: the 09:28 write time, two of attempt 2's three round-1 points already fixed in the revised copy, and the finalize plan still listing every question as open. The plan does not depend on them.
