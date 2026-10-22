Read CLAUDE.md, docs/design.md and docs/lessons.md first. Find the code the issue touches, name the files with line numbers as they are in this worktree, and say what the on-disk record and rehydration would be before what the pane looks like. End your notes with a section headed Open questions: every question a plan would have to settle that this tree does not answer, one per line, each tagged [tree] when a closer read of the tree would settle it, [source] when a document outside the tree would (a browser's behaviour, a platform's rules, a crate's API, a protocol; phrase these with the names a search would use: the browser, the mechanism, the failure), [run] when only running something would, and [owner] when only the owner can observe the answer (it needs a device, an account, a signed bundle, or the owner's own ticket). A design choice is not an open question: the plan makes those, and the issue's decisions belong to the planner, not to this list. A claim the issue makes about the world outside this tree is a question, not a finding, until something settles it; list it even when it sounds right. Do not settle the questions here: say what you did not check. A short list, or none, is a fine answer when the tree holds everything the plan needs. Write nothing but your notes, and write them before your last turn ends: a turn that ends with no notes file is read as a failure, however many background agents are still working for you. Never name a client or customer, or a client's repositories, environments or stacks, anywhere that reaches GitHub: code, tests, fixtures, docs, commit messages, PR bodies, issue text. Say the client project or a pipeline; the pipeline files under the data directory are the only place the real names live.

Issue #187: Dispatch: a review rerun after a failed attempt starts from the plan stage's original output, discarding the owner's revise

## What happened

A plan ticket on a client project reached the review stage's finalize decision and the owner answered `revise` with a note of lead decisions. The reviser wrote the revised plan to `review/1/joined/plan.md` (09:28). The review run then failed with `review run gone: no such run`, and the rerun decision was answered `rerun`.

Attempt 2 started one minute later from `plan/1/joined/plan.md`, the plan stage's original output, not from the revised copy in attempt 1. Its rounds converged and the finalize decision came back for a plan that still lists every question as open and contradicts one of the owner's answers. Two of attempt 2's three round-1 points were things the revised copy had already fixed.

Nothing warned that the revision had been discarded. `show` lists `review/1 joined failed` and `review/2 joined running`, and the files line names only the attempt-2 copy as the plan.

## Expected

A rerun after a failed review attempt should start from the newest plan the stage holds, which after a `revise` is the previous attempt's revised copy, or at least say in the rerun question and in `show` which copy it will review. The `revise` note is owner input and should survive a runner failure the same way an answered decision does.

## Where to look

- the review stage's rerun path: what it takes as the plan input for a new attempt;
- `show`'s `files:` block: when more than one attempt has a `plan.md`, say which is the reviewed copy and which is a prior revision.

## Repro

1. Take an issue through plan and review; at finalize answer `revise --note ...`.
2. Kill the review run so the attempt fails with `review run gone` (a runner restart mid-run does it).
3. Answer `rerun`.
4. Diff `review/1/joined/plan.md` against `review/2/joined/plan.md`: the revision is gone.


Investigate and write your findings to /private/tmp/claude-501/-Users-sully-code-repos-personal-switchboard/144a7def-c8e8-40f3-b0b2-5ef20bc546df/scratchpad/plan-trial/experiments/plan-trial/results/sb-187/stages/investigate/v3-sonnet-a/notes.md, ending with the Open questions section.