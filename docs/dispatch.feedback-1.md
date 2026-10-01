# Dispatch plan review

The separation between Dispatch policy and Switchboard process ownership is useful, but the plan needs changes before the first slice. Several capabilities described as existing do not yet support the proposed behavior.

## 1. Define durable execution and recovery before the socket

“Tickets are data” needs a concrete record and reconciliation contract. Specify stable ticket/source identity, stage and attempt IDs, lane paths and branches, Switchboard record/workflow IDs, artifact paths, decisions, resource ownership, and the pipeline/definition version used by a ticket. Define atomic persistence and single-writer ownership for Dispatch too.

In particular, cover a crash after Switchboard creates a session but before Dispatch records its ID, and a disconnect after a deploy starts but before its reply arrives. Restart must reconcile existing work without duplicate agents, duplicate deployments, or silently advancing incomplete gates. “Dispatch asks again” is not a sufficient distinction between authorized continuation and automatic paid resumption. State what a restart may observe, what it may continue, and what requires a fresh human decision.

## 2. The control port needs operation identity and results, not just action acceptance

`AddProject`, `NewSession`, `NewSpace`, and `StartWorkflow` create IDs inside Switchboard. An `{"ok": true}` reply cannot reliably associate the resulting records with a Dispatch ticket, especially with simultaneous UI activity. Core dispatch can also reject an action through a notice, while launch or persistence effects can fail separately.

Define request IDs, retry/deduplication behavior, returned object IDs, and a distinction between accepted, persisted, launched, and failed. Prefer a narrow wire command/read-model contract over exposing the complete internal `AppAction` enum, which also includes trusted adapter-result and startup actions. Validate the external allowlist before dispatch.

The proposed API also lacks space and working-set queries, a generic file probe, workflow-definition installation or discovery, and decision submission/readback. `SetSessionNotes` exists but is absent from the allowed actions. Map every operation needed by the first slice to an actual request and result; merely deriving serde does not implement that contract. Creation actions currently change UI selection and use the active space, so define explicit destination IDs and whether background Dispatch work should navigate the user's window.

## 3. The project-wide queue conflicts with workspace privacy

The plan puts every ticket in its own workspace, then places all tickets in one per-project working set. Existing working sets only accept targets from their own workspace; moving a project removes its pins from the old space (`src/core/action.rs`, `target_in`/working-set handling; `docs/design.md`, “Workspaces”). Workspaces are also a presentation boundary, not a filesystem access boundary between agents.

Choose a compatible layout or explicitly propose a new summary-card mechanism with its privacy rules. Existing `PinTarget` supports only sessions and project files, not tickets. Define a stable ticket card across fresh stage sessions and a deterministic queue order: a two-dimensional, resizable grid has no intrinsic total order, and `PlacePin` changes geometry rather than a queue rank. This is more Switchboard work than adding a socket.

## 4. Human decisions need a durable return channel

Putting options in notes and sending an answer “to the pane” delivers text to an agent, not to Dispatch. No described action sets an arbitrary session to waiting on a Dispatch decision or records the selected option. Stages such as `lanes`, `tried`, and `merge` may not even have a current running agent.

Specify decision IDs, pending/resolved state, allowed answers, and a machine-readable way for the user to submit an answer. Distinguish permission to perform an action from confirmation that the user already performed it: an automatically accepted `merge` or `publish` gate must not complete the ticket without actually merging or publishing. Define rejection/cancellation and delayed `recommend` behavior, including restart behavior and the configured delay. A Dispatch CLI decision command could be the first implementation if a generic Switchboard choice card is deferred.

## 5. Make the review integration match the existing workflow

The shipped definition is named `Plan review`, not `Built-in review`; `start_workflow` rejects unknown names (`src/core/model.rs`, `src/core/workflow.rs`). `PTA review` and `Orchard review` also need a provisioning mechanism. Explain how operator guidance becomes the definition's actual prompt templates; passing only a name does not apply it.

`StartWorkflow` requires an absolute plan path and a source Claude Code session with a discovered resume handle/transcript. The pipeline needs an explicit review subject and source session, plus readiness for cloning. The schema currently permits Codex planners, which the existing workflow cannot clone. PTA creates a draft and a notes file but never defines `{plan}` or identifies which artifact is reviewed; a mail draft or rendered PDF also needs a suitable reviewable source.

Finally, the workflow compares only the reviewer's no-feedback sentinel. Planner responses are prose and automatically lead to another round; rejected objections are not exposed as structured decisions. Define the additional mechanism if `review_objection = "ask"` must intercept them. `Converged` and `Finalized` are distinct states: state explicitly who finalizes and when, preserving the intended human review.

## 6. Separate stage completion from polling a successful command or stable file

As written, the implementation checks can pass on the untouched base while the agent is still starting. A successful exit must be tied to the completed stage attempt and the exact code being accepted, not merely to the lane's current last exit. Specify when a gate command launches, how its run ID is tracked, and what invalidates its result if files or commits change. Commands with side effects must run once per authorized attempt, not on every polling tick.

Likewise, the existing round-file probe observes metadata stability; it does not prove an agent finished. A reused `{notes}` or `{plan}` can already be settled before the new stage writes anything. Define stage/lane/attempt-scoped outputs and an explicit completion handoff, or an equivalent freshness protocol. Define artifact ownership and retention, including how parallel `each` stages avoid overwriting shared notes. Joined stages' “known commit” guarantee also needs to address dirty files and agents that remain active after their gate passes.

## 7. Define execution context before and after lane selection

Orchard's investigation says `lanes = ["backend"]` while its comment says it runs at the workspace root before lanes exist. Those are different contexts, and neither the schema nor the control flow explains how this stage starts. Introduce an explicit root/planning context, identify its Switchboard project and cwd, and explain how joined stages receive the selected lane roots and commits.

The `try` stage unconditionally uses `lane:backend`, although lane selection allows frontend-only or SNP-only tickets. Define whether backend is mandatory, how unchanged backend code is supplied, or how the stage behaves without it. Also describe how frontend worktrees receive environment linking and a running server: an existing service in the original checkout does not automatically run the ticket's frontend changes.

## 8. The Orchard deploy gate does not establish a tested, correctly targeted deployment

The tester prompt asks the agent to deploy, and the command gate deploys again. Successful deployment alone does not prove the end-to-end probes passed, and the second deployment may differ from what the tester examined. Choose one owner of deployment, then require test evidence and record the deployed revision before the human `tried` decision.

The command targets whichever environment is linked. A fresh worktree's `uv sync` does not establish that link, and a command-text approval hash cannot pin mutable linked-environment state. Define an explicit, verified target or fail-closed preflight for `my-dev`; explain how the claimed tool allowance is enforced or acknowledge that guidance is only advisory. Raising a resource count also does not create isolated environments: separate instances require allocation and binding of the chosen backend/frontend endpoints to each ticket.

## 9. Persist resource holds and define scheduler capacity

Keeping `my-dev` across `try` and `tried` is the right requirement, but specify atomic acquisition, ownership, release on rejection/cancellation, and recovery after either process restarts. Losing an in-memory hold while a human is still inspecting the stack would allow another ticket to overwrite it. State whether the lock covers only Dispatch tickets or also cooperating manual deploys.

Define whether tickets at human gates consume `slots`. If they do, PTA's `slots = 1` can never accumulate three waiting drafts. If they do not, its in-place repository still cannot be switched to another ticket branch while the previous ticket owns files there. Add a repository-level hold for in-place lanes, dirty-tree handling, and a clear rule for when work is safe to release. Specify backpressure when `waiting_on_me` is reached.

## 10. Replace PTA's commit gate

`git diff --quiet || git commit -am 'Dispatch: {task.text}'` succeeds when the only output is an untracked file, misses staged-only changes, can commit unrelated tracked edits, and does not validate a rendered PDF or saved draft. It also makes Dispatch's gate executor perform the commit despite the stated “only the agents commit code” boundary.

Have the writer explicitly stage/commit ticket-owned repository outputs, and make the gate verify the intended artifacts and any required commit. Specify how non-repository outputs such as mail drafts are identified. Never interpolate task text directly into shell source: apostrophes can break this command and crafted text can become executable syntax. Define argument-safe template expansion separately for commands and prompts.

## 11. Specify source identity and PR evidence

A task line's location or text is not a stable identity when the file is reordered or edited, and Dispatch never removes its marker. Define deduplication and explicit retake/reopen behavior for both task-file and GitHub sources. Persist the input snapshot used by a run.

Similarly, bind `pr-checks` to a specific PR, repository/provider, target branch, and head revision. Distinguish pending checks from failure, no PR, no required checks, authentication errors, and transient lookup failures. “Failure is a decision” should not turn normal CI waiting into an immediate failure or let old green checks certify new commits. Define what happens when one lane fails or is revised after other lanes have passed.

## 12. Budget enforcement needs an explicit accounting model

The existing transcript `Usage` contains token counts, not a complete dollar ledger for both agent kinds. The plan needs model/rate attribution, missing-usage behavior, and accounting that excludes cloned transcript history and does not double-count cumulative reads. Identify the project budget field and whether operator budgets are per stage, attempt, or ticket aggregate.

Specify whether a threshold stops a running agent, blocks the next launch, or only raises a notice. If precise live enforcement is deferred, describe budgets as approximate reporting rather than an enforced tool allowance or spending ceiling.

## 13. Tighten the first-slice acceptance criteria

Pick one concrete supported pipeline for the first slice, likely Switchboard. The advertised universal `investigate -> plan -> review` sequence does not match PTA's stages and skips Orchard's pre-worktree lane decision. The Switchboard prefix also has no explicit human gate unless finalization is counted; name the exact interaction being exercised.

Include a small acceptance matrix: successful issue-to-review flow; restart after a lost creation reply without a duplicate paid launch; unavailable planner transcript; stale artifact that must not advance a stage; launch/persistence failure; and workspace placement while the user is viewing another space. Later slices should test gate revision binding, durable resource ownership, and decision submission. This makes the architectural assumptions testable before queue automation and deployment are added.
