# Further feedback

The revised boundary, CLI decisions, shared project workspace, and deferral of objection interception and the recommend dial address the corresponding first-round concerns. The following execution gaps remain.

## 1. Fix the concrete workflow templates and freeze installed definitions

All three reviewer definitions say “the line below alone,” but neither `review_first` nor `review_round` includes `{no_feedback}` or the literal sentinel. `WorkflowDefinition::render` substitutes placeholders; it does not append the `no_feedback` field (`src/core/model.rs`). The reviewer therefore never receives the exact completion instruction. Include `{no_feedback}` explicitly in both templates, and include `{plan}` in later review prompts so they explicitly request a re-review of the updated subject.

A ticket retaining a pipeline version also needs that version's content to remain available. Store a snapshot or immutable versioned configuration. Install workflow definitions under versioned names or content hashes: existing workflow runs look up their definition by name each round, so re-installing an edited `Dispatch: ...` definition under the same name would silently change an in-flight ticket despite the promised version pinning.

## 2. Recovery tags must cover every creation and survive notes edits

Recovery queries use a request ID, while the description puts the attempt ID in session notes and puts command tags only in command names. `sessions.tagged` searches notes only. `workflow.start`, `project.add`, and `space.new` have no durable request tag at all. A crash after `workflow.start` succeeds but before Dispatch records its reply cannot be reconciled by the stated mechanism. Replacing notes to display a decision can also erase the recovery tag.

Define one immutable, exact-match correlation field or a durable operation ledger for all relevant creations, including the review run and its generated sessions. Distinguish the attempt ID from each operation/request ID. A found record is evidence that creation persisted, not that launch completed: a crash can occur between those two effects. Reconciliation must inspect that distinction rather than immediately label any found record `launched`. Conversely, an absent record is not proof that a side effect never occurred unless persistence-before-launch is guaranteed.

The wire protocol still says one reply per request while listing intermediate `accepted` and `persisted` outcomes without an operation-status query. Specify either a terminal reply with documented completion semantics or an operation query/event mechanism. Add recovery coverage for Switchboard restarting during workflow creation, not only Dispatch restarting during `session.new`.

## 3. Clarify stage outputs, review mutation, and stages without agents

The generic rule says every attempt creates a fresh session and completion always requires a stopped session plus an output. Several stages have no operator or output (`lanes`, `deploy`, `ready`, `merge`), while review creates a reviewer and a planner clone through an existing workflow. Explicitly distinguish agent stages, workflow stages, and gate-only stages, and define which completion predicates apply to each.

The output contract also needs named artifacts rather than two variables that appear to expand to the same `<context>.md`. PTA writes its reviewed subject in stage `draft`, but render reads `{inputs.plan}`; say whether `inputs` is keyed by stage or by artifact role, and show a valid binding. A skipped Orchard `deploy` leaves `{inputs.deploy.commit}` undefined: discover the actual backend revision or represent it as unknown and render the prompt accordingly.

Finally, “earlier stages' outputs are read-only inputs” conflicts with review intentionally editing the plan attempt's file. Define the review's writable subject and finalized output explicitly, preserving the original snapshot if it is intended to be auditable. Fresh review attempts also need distinct round-file locations: the current workflow derives feedback paths from the plan path and round number (`round_paths` in `src/core/workflow.rs`), so re-reviewing the same path can consume a prior attempt's settled feedback.

## 4. Do not use generic idle state as proof of completion

The revised rule says idle or exited means stopped, but Switchboard marks a live Codex pane idle after 20 seconds without output (`QUIET_AFTER` and `card_state`). That can happen while the agent is still working. Either constrain ordinary stage operators to a supported completion signal or require a request-scoped explicit completion event; a UI card state is insufficient.

Recovery currently uses a different rule: a missing session plus a written output is declared finished without evidence of successful completion or even settling. A process killed after writing a partial file must not become successful after restart. Persist and reconcile the same completion evidence used during normal execution, and distinguish successful exit from interruption/nonzero exit. Also name the work needed for generic stall detection: the existing stalled-agent mechanism is specifically for workflow runs, not arbitrary investigator or implementer sessions.

## 5. Bind checks and deployments to clean, unchanged inputs

Command gates now record both HEAD and dirty state, but invalidate results only when HEAD changes. Checks on dirty content can therefore certify a commit that does not contain the tested changes, and later uncommitted edits can change what gets deployed without moving HEAD. For revision-based gates, require a clean tree and check the revision and cleanliness before and after execution, or bind the result to an exact content snapshot instead.

Carry that evidence forward to PR creation and human approval. A manual push while a ticket waits to merge must invalidate the relevant local checks, deployment/test evidence, and approvals—not merely repeat `pr-checks`. The current rule only invalidates joined stages *after* `ready`, but Orchard's joined testing happens *before* `ready`; a changed head can otherwise merge with evidence from an older tested revision. Record the lane-head set associated with each dependent result and define which stages must be re-established.

## 6. Quiesce work before releasing holds

“Rejecting a decision cancels the attempt and releases its holds” needs a process-level meaning. Marking an attempt cancelled does not stop its agent, command, frontend server, or workflow automation. Releasing `my-dev` while a deploy still runs allows it to overwrite the next ticket's deployment; releasing an in-place repository while an agent can still write is similarly unsafe.

Persist cancellation intent, stop or pause all relevant writers, confirm their termination, and only then release ownership. A review must also be paused so its own tick cannot schedule another round; the proposed port currently lacks workflow pause/continue operations. If safe termination or repository cleanup cannot be confirmed, retain the hold and surface a decision. Apply this ordering to `retake` closing the old ticket as well.

## 7. Complete PTA's handoff before releasing its repository

The PTA explanation releases the repository after render, but the generic rule holds it from the first to the last stage in that lane, which includes `ready`. Choose an explicit release point. More importantly, the next ticket requires the checkout to be on `main`, while render leaves it on the completed ticket branch; define who restores the base and when.

Before that switch, preserve a stable publishable artifact outside the shared checkout or provide a branch-aware retrieval operation. Otherwise a PDF or guide awaiting human publication can disappear or change when another ticket checks out a different branch. The clean-tree/ahead-of-base gate still only proves that some commit exists; it does not verify the promised render or Drafts item. Require the artifact reference and appropriate existence/success evidence before declaring it ready.

## 8. Represent externally confirmed merge as pending human work

Changing merge to `pr-merged` correctly avoids pretending permission means completion, but it also removes its decision. As written, a PR awaiting manual merge has no pending decision, does not increment `waiting_on_me`, and need not appear waiting on the user. The scheduler can accumulate arbitrarily many merge-ready tickets despite the configured limit.

Keep a durable waiting confirmation for manual merge, resolved automatically when the provider reports the merge. Specify whether rebundling/backup is outside Dispatch's definition of done or requires a separate confirmation; a merged PR cannot verify those acts.

## 9. Correct the first-slice acceptance counts and background behavior

The happy-path test expects three sessions, but investigate and plan create two, and `StartWorkflow` creates both a reviewer and a planner clone, giving at least four (plus a placeholder if needed). Correct the test to assert the actual records and roles.

Background behavior must cover `workflow.start`, not just creation of projects/spaces/sessions: existing `start_workflow` explicitly selects the workflow view. Also specify whether Dispatch suppresses the user's open-terminal-on-launch preference for its launches or honors it; “the window does not change” is otherwise too broad. Test the complete issue-to-review path while another workspace is active.
