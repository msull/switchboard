# Further feedback

Most of the previous feedback is resolved. These remaining points affect the proposed execution contract and concrete pipelines.

## 1. Tell the cloned planner which copy to edit

Review now correctly copies the subject into a fresh attempt directory, but all three `respond` templates still say only “Update the plan/draft,” without `{plan}`. The cloned planner's history names the original file. The existing workflow sends the rendered `respond` text directly to that clone; it does not separately tell it that the subject moved (`prompt_planner` in `src/core/workflow.rs`). It can therefore edit the original while the reviewer keeps examining the unchanged copy. The same omission exists in `respond_to_user`.

Include the absolute `{plan}` path in both templates and explicitly direct the planner to edit that copy, leaving the earlier artifact unchanged. Add a first-slice case with at least one objection, asserting that the review copy changes and the original does not; convergence without objections would miss this failure.

## 2. Define recovery for commands that do not create records

The new operation field solves correlation for creations, but the port promises deduplication and terminal status for *every* command. `session.send`, `session.notes`, `session.waiting`, `workflow.continue`, and removals create no record on which to store a new creation operation ID. A removed record also cannot retain the operation that removed it. `find {op}` therefore cannot establish whether these commands ran, and repeating a lost `session.send` or `workflow.continue` can trigger duplicate paid work.

Specify a durable operation-result ledger, or explicitly divide commands into those recovered from resource state, those deduplicated durably, and non-replayable commands requiring a decision on an uncertain outcome. Retain enough operation history after removal to satisfy the advertised behavior. Scope “not found means nothing ran” to cases where records cannot have been removed; persistence-before-launch alone does not make that statement true after deletion.

Also distinguish an in-progress creation from an interrupted one. Dispatch restarting while Switchboard is still cloning a planner can legitimately find a run in `Starting` with no clone. Consult `op.status` and wait for the active operation; only declare it interrupted once Switchboard establishes that no creation effect remains in progress. The current rule fails it after either process restarts, including a Dispatch-only restart.

## 3. Separate normal multi-lane progress from invalidation and retries

“Every result” captures the entire lane-head set, and any head movement returns the ticket to the earliest affected stage. During ordinary parallel implementation, backend checks can finish before frontend commits; that expected frontend commit then invalidates backend evidence and sends the ticket backwards while the original frontend attempt is still running. The text also says new attempts require a decision, whereas the invalidation paragraph appears to rerun checks, deployment, and paid tester sessions automatically.

Define the barrier and authorization rule. One workable version is to wait for all implementation agents to finish, capture a stable head set, and then run the lane checks; another is to bind independent checks only to their actual dependencies and bind joined results to all lanes. On later movement, explicitly park for a decision or state which reruns were authorized, retire superseded attempts, and prevent concurrent old and replacement attempts. Test one lane committing after another lane's checks have finished.

## 4. Make PTA's manifest validation fail closed and specify its format

The new shell gate ends in `grep ... | while ...`. With ordinary `sh` pipeline status, an unreadable manifest makes `grep` fail but the empty `while` loop return zero; an empty manifest also passes. Thus this command does not establish that a usable deliverable list exists. The prompt requests a mail's subject line, but the parser silently assumes the undocumented `Drafts:` prefix, so a conforming plain subject can instead be treated as a nonexistent path.

Specify file versus Drafts entries explicitly and require at least one valid entry. Prefer a small parser/validator, or check readability and non-emptiness separately and propagate every read/parse failure. Validate file entries as repository-contained regular files before copying them, preserve their relative paths, and persist the completed copy before restoring the checkout and releasing the hold. Include tests for an unreadable/empty manifest, a missing file, a mail-only manifest, and two deliverables with the same basename.

## 5. Extend quiescence across the entire resource hold

The cancellation sequence kills sessions and commands owned by the *current attempt*. By `tried`, the current attempt is gate-only; the frontend server belongs to the earlier `try` attempt. Earlier agent panes can also remain alive after their Stop event. The Resources section's requirement to stop every possible writer is stronger than the cancellation sequence it references.

Track processes across the ticket's entire hold, including services retained from earlier stages, and define how the tester's frontend server becomes a Switchboard-owned process with an ID. A background process launched inside the tester's shell is not automatically represented by a separate session record or guaranteed dead when the tester pane disappears. Either have Dispatch start the server through the port or provide an explicit registration and shutdown contract. Release and cancellation must verify all those owned processes, not only the current attempt, before allowing another ticket to use the resource.

## 6. The queue view needs a pin removal/replacement operation

Surfacing promises to replace a ticket's old session card when a stage changes and to rearrange cards when the queue changes. The port exposes only `set.add`; there is no unpin, clear, replace, or explicit move operation. Existing `AddToWorkingSet` ignores a target already present, and placement rejects overlaps (`src/core/action.rs`), so merely re-adding cards cannot implement this view.

Add an explicit set-reconciliation operation or the necessary remove/place operations, with idempotent semantics. Demonstrate replacing the investigator with the planner and swapping two queue positions without leaving stale cards or losing cards on overlap. This is required by the current surfacing plan, even with a dedicated ticket card deferred.
