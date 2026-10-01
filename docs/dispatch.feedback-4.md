# Further feedback

The previous round's changes resolve the template, manifest, multi-lane invalidation, and queue-layout findings. Two gaps remain in the revised recovery and service contracts.

## 1. Make the record-retention assumption true, or treat disappearance as uncertain

Recovery now relies on “only Dispatch removes records” to conclude that an absent creation record means nothing ran. That restriction is not enforced or specified for the Switchboard UI. The plan otherwise says these are ordinary records visible in the real window; existing UI actions can remove sessions, workflows, and working sets (`src/ui/cards.rs`, `src/ui/workflow.rs`, and `src/ui/dialogs.rs`). A user can therefore remove a record after its creation succeeds but before Dispatch durably records the reply.

Choose an explicit policy: prevent direct deletion of Dispatch-owned records while their operation history is needed, retain durable creation tombstones after deletion, or classify an absent record as an uncertain outcome rather than proof of non-execution. If deletion is restricted, state how the UI explains it and how the user removes the item through Dispatch. Add a recovery case where a creation succeeds, its reply is lost, and its record is removed through the UI before reconciliation. No inference that paid work or a side effect never occurred should depend on an unenforced convention.

## 2. Complete the new Orchard service binding

The `try` stage now unconditionally requests `services = ["frontend"]`, runs `before.frontend`, and expands `{services.frontend.url}`. Lane selection still allows backend-only and SNP-only tickets, for which no frontend project/worktree exists. Unlike the backend deploy, there is no stated skip or fallback rule for this service dependency.

Define whether frontend is required, supplied from an explicit unchanged checkout, or omitted when absent, and render the tester prompt accordingly. Cover backend-only, frontend-only, and SNP-only tickets so none references an uncreated lane.

For tickets that do have a frontend, specify where the service URL comes from and how readiness is established. `serve` currently supplies only argv and environment, and `service.new` returns a session ID; neither defines the URL used by the prompt. A successful process launch also does not mean the ticket's server is listening, especially if a manual server already occupies its default port. Add an explicit address/port binding and a readiness check tied to that service, with startup failure becoming a decision before the tester runs. Execute the `before` command as a tracked operation and require it to succeed before starting the service.
