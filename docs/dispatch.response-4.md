# Response to review 4 of the Dispatch plan

Both claims were checked: sessions, runs and working sets can be
removed from the window (`src/ui/cards.rs`, `src/ui/workflow.rs`,
`src/ui/runs.rs`, `src/ui/dialogs.rs`), and the Orchard admin frontend
is a Create React App that owns port 3000 by default and takes `PORT`
from the environment. Both items are accepted.

## 1. Record retention — accepted, with tombstones

The convention is replaced by a mechanism. Before running any effect
for a creation, Switchboard appends `{op, kind, ids, time}` to an
append-only `operations.log` in its data directory and persists the
record; removing the record in the window never touches the log.
`find {op}` answers from the log first, so a removed record is
reported as `made, removed` rather than absent, and only an `op`
missing from the log means nothing ran. Removal in the window stays
allowed: Dispatch-owned records carry a visible mark and name their
ticket in their notes, and a hand removal fails the attempt as
`removed by hand` with a decision, treating any pane it killed as
stopped without completion evidence. The first slice gained the case
asked for: a creation whose reply was lost and whose record was
removed before Dispatch restarted, expecting no second launch.

Restricting deletion was not chosen: it would need a new refusal path
in four UI surfaces and an explanation in each, for a case the log
already makes safe.

## 2. Orchard service binding — accepted

Services are now per lane and conditional. `services` names lanes,
each started only if the ticket cut it, and a template field for one
that was not renders as `not served (no <lane> lane)`; the `try`
prompt lists both the admin frontend and the student portal that way
and tells the tester an unserved lane is tested against the existing
deployment if at all. The portal lane gained its own `serve` entry.

For a lane that was cut, the order is stated: the `before` command
runs as a tracked `command.run` operation and must exit zero; Dispatch
allocates a port from a `ports` range in the policy, testing that it
binds before choosing it, so a hand-started server on the default port
is never picked; the service starts through `service.new` with
`{port}` filled into `PORT` and the URL; and readiness is an HTTP
probe on that URL within a limit. A `before` failure, no free port, or
a probe that never answers is a decision before the tester launches.
The port's `service.new` reply is stated to mean the process started,
not that it listens; listening is Dispatch's probe.
