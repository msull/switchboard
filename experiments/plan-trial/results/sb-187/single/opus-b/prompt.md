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


Write the plan the way docs/design.md sections are written: a heading Where this departs from the issue first (each claim in the issue the code does not bear out and what the plan does instead, or the one line Nothing), then the record, the signals, the actions and effects, the UI, the tests. One commit, README and design status updated, the app runs at every step. Every file and line reference is checked against the worktree you are in, not the notes; name the schema versions as they are today. A version number in a plan is a snapshot: write the bump as: from the value in store.rs or model.rs at implementation time, plus one, because a ticket landing in parallel may take the next number first. Size the plan to the change: a section with nothing to say (no record change, no new signal, no UI) is one line saying so, not a heading with prose under it; a small issue gets a short plan. End with a one-line commit message and the list of contract additions, which the implementer copies. Never name a client or customer, or a client's repositories, environments or stacks, anywhere that reaches GitHub: code, tests, fixtures, docs, commit messages, PR bodies, issue text. Say the client project or a pipeline; the pipeline files under the data directory are the only place the real names live.

Read the repository first. Then write the plan to plan.md in the current directory, and write nothing else.
