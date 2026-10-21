# Where a secret value from an environment set travels (#217)

**Provenance.** Written 2026-10-20 against `msull/switchboard` at
`70b2074` (Release 0.3.0). Every file and line reference below is to
that commit.

**Question.** Where can a secret value from a Switchboard environment set
go between the Keychain and a process? For each hop, can the value reach
a record, a log line, a shell command line, or a wire reply, and does
code prevent it (guarded) or only convention (unguarded)?

**Answer.** On the way to a process, a set's value travels only as the
`env.resolve` reply to `switchboard-env`, which puts it in one child's
environment. Every hop up to that child is guarded with two exceptions,
both about who can ask for that reply: a process that can reach the
tmux server can take another session's launch token, and most children
of the Dispatch runner inherit the runner's token (the first two
findings below). One more hop is safe today by convention only: the
wire and policy types derive `Debug` with the value as a field, so a
future log line or error that formats an `env.resolve` reply would
print it; see
[`env.resolve`](#envresolve-on-controlsock-to-switchboard-env). What
the child prints is not guarded: it can reach the agent's tool result,
scrollback, `events.log` and a command gate's `checks.log`, none of
which is masked. `session.screen` and the remote session views mask
it, but the remote port sends `checks.log` unmasked (the third finding
below).

Three findings contradict `docs/design.md`:

- Any process that can reach the tmux server can read another session's
  launch token and record id with `tmux show-environment` and resolve
  that session's grants, so "a session resolves only its own grants"
  (`docs/design.md:244-248`) does not hold against a process in another
  pane. See
  [The launch token](#the-launch-token-which-unlocks-envresolve).
- Every child of the Dispatch runner not built by `child_command`
  inherits the runner's launch token: its `git` commands and the hooks
  git runs for them, `gh`, `curl`, `df`, `ps` and `kill`. Any of them
  can run `switchboard-env exec` with the runner's grants, though
  design.md says every child of the runner but an `env` gate has the
  token removed (`docs/design.md:2443-2447`). See
  [Dispatch's grants](#dispatchs-operator-and-stage-env-grants).
- A command gate's `checks.log` is sent to a paired remote device with
  no redaction, though design.md names redacted text as the only other
  way a secret's surroundings leave the app (`docs/design.md:250-254`).
  See [Dispatch's grants](#dispatchs-operator-and-stage-env-grants).

In more detail: a set's value leaves the Keychain on two code paths, the
`env.resolve` reply on `control.sock` and the redaction policy the app
builds in memory to mask text. It never travels the `session.new` launch
through tmux's stdin; that path carries the global and project layers,
the launch token, and set *names*. Dispatch's operator and stage `env`
grants are names too; at run time they reach a value only through
`switchboard-env exec`.

Paths are relative to the repository root. Each hop carries one
verdict:

- **Guarded**: code makes the leak impossible, or refuses it, on that
  path.
- **Names only**: the hop carries set or variable names, never a value.
- **Intended**: the hop delivers the value where the hard rules allow
  it to go; nothing guards it because it is the destination.
- **Unguarded**: nothing in code stops the leak; only convention or the
  owner's accepted risk does.

A bullet marked **Context** is not a hop: it states a fact the next
verdict rests on. A claim marked **Unverified** is one this document
could not confirm from the code or by running something.

## What to act on

Unguarded, and not yet in `docs/design.md`: the three findings above,
plus:

- A code review round with no reviewer writes `checks.log`, set values
  included, into the lane's working tree, where it can be committed and
  pushed; only a hand-edited ticket reaches it today. See
  [Dispatch's grants](#dispatchs-operator-and-stage-env-grants).
- A value from another session's set is not masked by this session's
  policy. See [What the child prints](#what-the-child-prints).
- A Keychain error on a set or global secret while the policy is built
  leaves that value unmasked; only a project secret's error fails the
  policy. See [What the child prints](#what-the-child-prints).
- Redaction matches the literal value only; an encoded or partial value
  passes. See [What the child prints](#what-the-child-prints).
- The wire and policy types derive `Debug` with the value as a field,
  and Dispatch already formats `switchboard_control::Reply` with `{:?}`
  in error messages and a warning log line. See
  [`env.resolve`](#envresolve-on-controlsock-to-switchboard-env).
- Dispatch's `output()` formats a lane `setup` command with `{:?}`,
  environment included, into an error that is logged and stored on the
  ticket. See [Dispatch's grants](#dispatchs-operator-and-stage-env-grants).
- `events.log` keeps up to 200 characters of the agent's last message
  and the whole `error` field. See
  [What the child prints](#what-the-child-prints).
- The record's `env` field accepts a value although it is documented as
  names only. See
  [The `session.new` launch](#the-sessionnew-launch-through-tmuxs-stdin).
- `switchboard-env set var` stores whatever it is given as a plain
  variable, from argv. See
  [Getting the value in](#getting-the-value-in-switchboard-env-set-secret).

Unguarded, and already a known gap in `docs/design.md`:

- `env.resolve` writes no audit line (`docs/design.md:2623`).
- Scrollback holds whatever a child prints (`docs/design.md:2630`).
- The `checks` log holds whatever the command prints
  (`docs/design.md:2540`).
- The runner's grants serve every `env` gate it runs
  (`docs/design.md:2626-2627`); a matter of scope, not of travel.

## Where the value is stored

- A set's secret is stored in the Keychain under the account
  `set/<set>/<VAR>`. Guarded. (`src/core/env.rs:24-33`)
- `env.secret.store` writes the variable into `settings.json` (a record)
  with an empty value, and the value travels on only as
  `Effect::StoreSecret`, which the app hands to the Keychain adapter.
  Guarded. (`src/core/control.rs:553-577`, `src/app.rs:585-589`,
  `src/adapters/keychain.rs:63-69`)
- `put_var` clears the value of any secret variable before it reaches
  `settings.json`, so turning a plain variable secret blanks what the
  file held. Guarded. (`src/core/control.rs:966-976`)
- `env.set.upsert` that carries a secret with a value is refused.
  Guarded. (`src/core/control.rs:539-541`)
- The `env.sets` reply blanks every secret's value. Guarded.
  (`src/core/action.rs:2630-2655`)
- Keychain errors carry `e.to_string()` and the account name, never the
  value. Guarded. (`src/adapters/keychain.rs:49-69`, `src/app.rs:585-589`)

## Getting the value in: `switchboard-env set secret`

- The value is read from the terminal with echo off, or as one line of
  stdin; never from argv. Guarded.
  (`src/bin/switchboard-env.rs:110-117`, `src/bin/switchboard-env.rs:392-420`)
- It is sent as `env.secret.store` on `control.sock`, which is created
  with mode 0600. Guarded, by uid.
  (`src/bin/switchboard-env.rs:163-172`, `src/adapters/control.rs:133-138`)
- The app accepts it only inside the five-minute setup window the owner
  opens from the settings menu. Guarded.
  (`src/core/control.rs:493-498`, `src/core/control.rs:512-521`,
  `src/core/action.rs:955`, `src/core/action.rs:1239-1241`,
  `src/ui/switcher.rs:85`)
- The request never reaches the operations log. `env.secret.store` is
  `Class::Idempotent`; only a `NonReplayable` command writes a
  `Requested` line, and the `Replied` line holds the reply (`persisted`
  with the records made), not the request. The refusals in
  `env_secret_store` name the set and variable only. Guarded.
  (`control/src/lib.rs:435-437`, `src/app.rs:3792-3811`,
  `src/app.rs:3816-3843`, `src/ports/control.rs:14-29`,
  `src/core/control.rs:554-564`)
- `switchboard-env set var <set> NAME=VALUE` puts the value on argv and
  stores it in `settings.json` as a plain variable. Unguarded: it is
  meant for plain variables, and nothing stops an owner from storing a
  secret this way. (`src/bin/switchboard-env.rs:96-109`)

## The `session.new` launch through tmux's stdin

Set values do not travel this hop.

- `session.new` and `session.clone` carry `env_sets: Vec<String>`, and
  the core copies the names onto the record. Names only.
  (`control/src/lib.rs:184-187`, `control/src/lib.rs:205-208`,
  `src/core/control.rs:277-281`, `src/core/control.rs:291-303`)
- A pane's environment is the data directory, then
  `resolve_project_env` (global layer, the project's `.env` files when
  opted in, the project layer), then the record's `env`, then the
  launcher's own, then a fresh token. `spawn_env` never calls
  `resolve_sets`. Guarded.
  (`src/app.rs:995-1016`, `src/app.rs:1022-1053`, `src/app.rs:111-136`)
- `resolve_sets` has two callers outside tests: `env.resolve` and the
  redaction policy. The module doc says set values "never go into a
  pane's environment". Guarded.
  (`src/app.rs:3997`, `src/content.rs:208`, `src/core/env.rs:6-7`)
- What does go into the pane (global and project secrets, the token, the
  pane command) is written to the tmux client's stdin as a `new-session`
  line through `start-server ; source-file -`, never argv. A test checks
  that the server's argv lacks a marker value. Guarded.
  (`src/adapters/tmux.rs:22-27`, `src/adapters/tmux.rs:197-224`,
  `src/adapters/tmux.rs:314-326`, `src/adapters/tmux.rs:584-612`,
  `src/adapters/tmux.rs:676`, `src/adapters/tmux.rs:1191-1222`)
- The spawn's debug log line prints variable names only. Guarded.
  (`src/app.rs:1045-1049`)
- A failed spawn logs tmux's stderr. Set values are not on the line, so
  for them this hop is guarded; for global and project secrets it is
  not settled. Three malformed lines (see Checks run) did not echo the
  `-e` value. Whether any tmux parse error can echo part of the line is
  **Unverified**: three cases are not all of them.
  (`src/app.rs:1011-1014`, `src/adapters/tmux.rs:216-221`)
- The record's `env` field, which `session.new`'s `env` map fills, is
  documented as "paths and names, never secret values", but nothing
  refuses a value there, and it is written to the record and replayed on
  every spawn. Dispatch fills it only with `DISPATCH_INPUT_*` paths. No
  set value goes there today. Unguarded, by convention.
  (`src/core/control.rs:279`, `src/core/model.rs:1151-1154`,
  `dispatch/src/scheduler.rs:9297-9299`)

## The launch token, which unlocks `env.resolve`

- Each spawn makes a fresh token from two v4 uuids, drops any token a
  launcher carried, and records only its SHA-256 on the record. Guarded:
  the record never holds the token.
  (`src/app.rs:998-1010`, `src/core/env.rs:46-57`,
  `src/core/model.rs:1158-1162`)
- Context: the token and the record id both go into the pane as tmux
  *session* environment (`new-session -e`). (`src/app.rs:1010`,
  `src/core/reconcile.rs:330`, `src/core/reconcile.rs:342-343`,
  `src/adapters/tmux.rs:601-604`)
- Context: any client of the same tmux server can read another
  session's environment back. A command in session `sb-b`'s pane read
  both of session `sb-a`'s values (see Checks run). A pane's own `$TMUX`
  names the socket, and host session names are `sb-<uuid>`
  (`src/core/model.rs:901-905`).
- `env.resolve` checks only that the token's hash matches the record's;
  nothing ties the request to the calling process. A process in pane A
  (an agent's Bash tool, for example) can therefore read session B's
  token and id, or the Dispatch runner's, and run `switchboard-env exec`
  as B with B's grants. **Unguarded.** (`src/app.rs:3960-3969`;
  the runner's own credentials are its pane's,
  `dispatch/src/scheduler.rs:155-170`)
- Context: this contradicts `docs/design.md:244-248`, which says "a
  session resolves only its own grants" and that "the token is as
  private as the pane's environment". That holds only against a process
  that cannot reach the tmux server.
- Whether Claude Code, as Dispatch launches it, would prompt before an
  agent runs `tmux show-environment` is **Unverified**. Spike 14's check
  (e), the related question for `switchboard-env exec`, is also unrun
  (`docs/design.md:2604-2610`).

## `env.resolve` on `control.sock` to `switchboard-env`

- The socket is owner-only (mode 0600). Guarded, by uid.
  (`src/adapters/control.rs:133-138`)
- The app answers only the token whose hash matches. It refuses an empty
  token and a record launched before tokens existed. Guarded, but see
  [The launch token](#the-launch-token-which-unlocks-envresolve).
  (`src/app.rs:3960-3969`)
- Values are read from the Keychain at request time and never cached. A
  Keychain error is treated as a missing value, and `switchboard-env
  exec` refuses to run when any value is missing. Guarded, fails closed.
  (`src/app.rs:3987-4003`, `src/app.rs:3996`,
  `src/bin/switchboard-env.rs:253-258`)
- `env.resolve` is a query (`Class::Query`), and `serve` answers a query
  before any operations-log write. Guarded, by structure.
  (`control/src/lib.rs:447-459`, `src/app.rs:3777-3780`)
- The one log line names the host session and counts the variables.
  Guarded. (`src/app.rs:3972-3976`)
- The reply is the wire `Reply::Env { pairs, aws, missing }`, the one
  wire reply the hard rules allow a value to travel in. Intended.
  (`control/src/lib.rs:841-850`, `src/app.rs:3977-3981`)
- The remote (HTTPS) port cannot ask `env.resolve`, `env.sets` or
  `env.secret.store`. Its route table builds a fixed set of bodies, and
  its `session.new` always sends an empty `env_sets`. Guarded.
  (`src/app.rs:3044-3110`, `src/app.rs:3311-3322`)
- `Request`, `Body`, `Reply`, `SetsResolved`, `Policy` and `Effect` all
  derive `Debug` with the value as a field, and nothing stops a log line
  or error message from printing one. Dispatch already formats
  `switchboard_control::Reply` with `{:?}` when a reply is not the one
  it expected: in `anyhow` contexts, `bail!`s, a health result, and one
  `log::warn!` (`scheduler.rs:7454`), which puts a whole `Reply` into a
  log line. None of those is an `env.resolve` reply today: only
  `switchboard-env` sends `env.resolve`, and its unexpected-reply arm
  prints a fixed string. But nothing stops a future `Reply::Env` from
  reaching one of those sites. **Unguarded.** Dispatch's
  `RunnerCredentials` writes `Debug` by hand to leave the token out,
  which is the guarded pattern.
  (`control/src/lib.rs:113`, `control/src/lib.rs:155`,
  `control/src/lib.rs:793`, `src/core/env.rs:60`, `src/content.rs:241`,
  `src/core/action.rs:719`, `dispatch/src/scheduler.rs:159-166`;
  Dispatch's `{:?}` of a `Reply`: `dispatch/src/supervisor.rs:941`,
  `dispatch/src/supervisor.rs:1067`, `dispatch/src/scheduler.rs:2711`,
  `dispatch/src/scheduler.rs:5930`, `dispatch/src/scheduler.rs:5982`,
  `dispatch/src/scheduler.rs:6780`, `dispatch/src/scheduler.rs:7454`,
  `dispatch/src/scheduler.rs:7592`, `dispatch/src/scheduler.rs:8248`,
  `dispatch/src/health.rs:246`, `dispatch/src/recover.rs:175`,
  `dispatch/src/recover.rs:213`, `dispatch/src/recover.rs:357`,
  `dispatch/src/view.rs:70`; `switchboard-env`'s arm:
  `src/bin/switchboard-env.rs:183-195`)
- No line records who resolved what. Unguarded; known gap.
  (`docs/design.md:2623`)

## `switchboard-env exec` to the child

- The resolved pairs go only into `Command::envs`, and the process
  `exec`s the child. Guarded.
  (`src/bin/switchboard-env.rs:203-215`, `src/bin/switchboard-env.rs:247-288`)
- The child never inherits the launch token or a stale `AWS_VAULT`;
  when a set gives an AWS method, every ambient AWS credential and
  profile is removed too. Guarded.
  (`src/bin/switchboard-env.rs:27-44`, `src/bin/switchboard-env.rs:199-212`)
- `check` prints variable names, the AWS method and the profile; `sets`
  prints plain values and `(secret)` for a secret. Guarded.
  (`src/bin/switchboard-env.rs:290-354`, `src/bin/switchboard-env.rs:356-381`)
- Error lines carry names (missing secrets, absent static keys) and the
  program name, never values. Guarded.
  (`src/bin/switchboard-env.rs:253-258`, `src/bin/switchboard-env.rs:272-279`,
  `src/bin/switchboard-env.rs:285-287`)
- `vault` wraps the child in `aws-vault exec <profile> --`; `sso` checks
  the session with `aws sts get-caller-identity --profile <p>`. The
  profile is on argv; the values are in the environment. Guarded.
  (`src/bin/switchboard-env.rs:217-230`, `src/bin/switchboard-env.rs:259-271`,
  `src/bin/switchboard-env.rs:331-341`)
- Another process of the same uid can read the child's environment
  (`ps -E`; see Checks run). Unguarded; the hard rules accept that a
  granted child holds the values.

## What the child prints

- An agent's Bash tool result, and so the provider's transcript.
  Unguarded, accepted by the owner when granting a set.
  (`docs/design.md:257-259`)
- Scrollback: every pane's output is piped with `pipe-pane` to `cat >>`
  a file under the data directory. Unguarded; known gap.
  (`src/app.rs:996`, `src/adapters/tmux.rs:254-263`, `docs/design.md:2630`)
- `session.screen` on `control.sock` masks the text with the session's
  policy and refuses to answer if the policy cannot be built. Guarded by
  literal-match redaction. (`src/app.rs:4009-4040`, `src/app.rs:4024-4031`)
- The remote port's screen, conversation, last response, run output and
  project files are masked with the policy. Guarded by literal-match
  redaction. (`src/app.rs:3099-3107`, `src/content.rs:572`,
  `src/content.rs:615`, `src/content.rs:678`, `src/content.rs:703`)
- The policy covers only the sets granted to this session and its
  project, so a value from another session's set (one obtained as in
  [The launch token](#the-launch-token-which-unlocks-envresolve)) is not
  masked. **Unguarded.**
  (`src/app.rs:1914-1930`, `src/content.rs:208`)
- Redaction replaces the literal value, also across line breaks, and
  nothing else: an encoded, partial or transformed value (base64,
  URL-encoded, truncated) passes. **Unguarded.**
  (`src/content.rs:994-1008`, `src/content.rs:1017`)
- While the policy is built, every Keychain read goes through one
  `lookup` that logs an error with the account name and treats the value
  as absent, so it is not masked. That covers global, project and set
  secrets alike. Afterwards only the *project* secret accounts are read
  again, and an error there fails the whole policy. "Nothing is sent
  unredacted" therefore holds for project secrets only; a global or set
  secret whose read errors is silently left out of the mask.
  **Unguarded.**
  (`src/content.rs:184-192`, `src/content.rs:193-200`,
  `src/content.rs:207-218`, `src/content.rs:219-234`)
- `switchboard-hook` writes each hook event to `events.log`, copying the
  payload's `error` field whole and `last_assistant_message` up to 200
  characters. If the agent repeats a value in its message, up to 200
  characters of it land in `events.log`. **Unguarded.**
  `PostToolUseFailure` is one of the hooks Switchboard registers. Whether
  its `error` field carries the failed command's output is
  **Unverified**: no spike records that payload.
  (`src/bin/switchboard-hook.rs:28-40`, `src/bin/switchboard-hook.rs:79-81`,
  `src/bin/switchboard-hook.rs:125-137`, `src/adapters/hooks.rs:46`)

## Dispatch's operator and stage `env` grants

- `Operator.env` and `Stage.env` are lists of set names; `env_sets`
  takes their union, operator first. Names only.
  (`dispatch/src/pipeline.rs:362-366`, `dispatch/src/pipeline.rs:553-558`,
  `dispatch/src/pipeline.rs:369-386`)
- An operator that is not Claude Code is refused `env` at validation.
  Guarded. (`dispatch/src/pipeline.rs:1257-1261`, `docs/design.md:2621-2622`)
- An agent launch puts the names on `session.new` or `session.clone`
  `env_sets`; the prompt gets one sentence naming the absolute
  `switchboard-env` path and no value. The pane gets no set value, so the
  agent reaches one only through `switchboard-env exec`, covered above.
  Names only.
  (`dispatch/src/scheduler.rs:6221-6266`, `dispatch/src/scheduler.rs:6281-6298`,
  `dispatch/src/scheduler.rs:212-220`)
- `switchboard-env grant --runner` adds the set name to the runner's own
  session record. Names only.
  (`src/bin/switchboard-env.rs:143`, `src/core/control.rs:602-610`)
- A command gate in a stage with `env` runs as `switchboard-env exec --
  <argv>`, with the runner's record id and token in the child's
  environment, not on argv. `switchboard-env` then drops the token from
  the gate command's own environment. Guarded.
  (`dispatch/src/scheduler.rs:6300-6329`, `dispatch/src/git.rs:420-421`,
  `src/bin/switchboard-env.rs:206`)
- Only a child built by `child_command` loses the runner's token: a
  lane's `setup`, a confined run, a command gate and a command reviewer.
  `git()` removes only the caller's `GIT_*` variables, and nothing in
  `dispatch/src` calls `remove_var`, so the token stays in the runner's
  own environment, where `RunnerCredentials::from_env` reads it, and
  every other child inherits it: each `git()`/`git_in()` command, with
  the hooks git runs for it (post-checkout on `worktree add`, pre-push on
  `push_with_lease`), `gh`, `curl` for Bitbucket, `df`, `ps`, `kill` and
  `confine`'s `git rev-parse`. Any of them can run `switchboard-env exec`
  as the runner with the runner's grants. Git takes hooks from the
  clone's own `.git/hooks` or a `core.hooksPath` in its config, never
  from tracked content, so a hook runs with the token only if something
  on the machine installed it there, such as a lane `setup` step that
  installs hooks or an agent that writes into the shared git directory.
  **Unguarded**, and contrary to
  `docs/design.md:2443-2447`, which says every child of the runner but
  an `env` gate has the token removed.
  (`dispatch/src/git.rs:367`, `dispatch/src/git.rs:379`,
  `dispatch/src/git.rs:479-490`, `dispatch/src/git.rs:508-519`,
  `dispatch/src/git.rs:632`, `dispatch/src/git.rs:646`,
  `dispatch/src/git.rs:835-840`, `dispatch/src/git.rs:864`,
  `dispatch/src/scheduler.rs:171-175`, `dispatch/src/github.rs:200`,
  `dispatch/src/github.rs:223`, `dispatch/src/github.rs:239`,
  `dispatch/src/github.rs:275`, `dispatch/src/github.rs:399`,
  `dispatch/src/bitbucket.rs:93`, `dispatch/src/confine.rs:208`)
- `switchboard-env` runs outside the sandbox so it can reach
  `control.sock`; the confined command receives the values in its
  environment. Intended.
  (`dispatch/src/git.rs:187-191`, `dispatch/src/git.rs:420-421`)
- The `GateRun` saved on the ticket keeps the stage's `argv` and the log
  path, not the outer wrapper or the environment. Guarded.
  (`dispatch/src/scheduler.rs:6872-6882`)
- `output()` formats its `Command` with `{cmd:?}`, and Rust's `Command`
  `Debug` prints the environment changes as well as the argv.
  `GitCli::run` hands it a lane's `setup` command built by
  `child_command(program, env)`, so a failed setup puts that `env` into
  the error, which the scheduler logs and stores as the ticket's reason.
  Today that `env` is `env_for`'s `DISPATCH_TICKET`, `DISPATCH_LANE` and
  `DISPATCH_BRANCH`, and `child_command` removes the token, so no secret
  is printed. But nothing stops a future set value or token in that
  `env` from being printed whole. **Unguarded.**
  (`dispatch/src/git.rs:527-531`, `dispatch/src/git.rs:1018-1026`,
  `dispatch/src/git.rs:483-490`, `dispatch/src/scheduler.rs:4071-4083`,
  `dispatch/src/scheduler.rs:10993-11006`)
- Every other `{cmd:?}` in Dispatch's non-test code is on a `git`
  command from `git()` or `git_in()` (`git()` without `-C` for the clone
  at `git.rs:547`), or on `df -k` (`git.rs:864`). These children
  inherit the token (above), but `Command`'s `Debug` prints only the
  variables set on the `Command`, not inherited ones, and none of these
  sets the token or a set value; the only one that sets variables,
  `commit_tree`, sets author and committer names. So none prints the
  token. A confined run's error formats its `argv` only, and a gate's
  start error names the program and directory only. Guarded, for
  printing.
  (`dispatch/src/git.rs:471-473`, `dispatch/src/git.rs:510-525`,
  `dispatch/src/git.rs:547`, `dispatch/src/git.rs:753-756`,
  `dispatch/src/git.rs:765-770`, `dispatch/src/git.rs:782`,
  `dispatch/src/git.rs:799`, `dispatch/src/git.rs:819`,
  `dispatch/src/git.rs:840`, `dispatch/src/git.rs:854`,
  `dispatch/src/git.rs:864`, `dispatch/src/git.rs:1007-1010`,
  `dispatch/src/git.rs:1047-1051`, `dispatch/src/git.rs:1411-1418`,
  `dispatch/src/git.rs:1432-1452`, `dispatch/src/git.rs:1457-1482`)
- The gate command's stdout and stderr both go to `checks.log` in the
  attempt directory. **Unguarded**; known gap.
  (`dispatch/src/scheduler.rs:6826`, `dispatch/src/scheduler.rs:6849-6860`,
  `dispatch/src/git.rs:415-423`, `docs/design.md:2540`)
- The code review stage's checks run through the same `gate_env` and
  write `checks.log` in the round directory, the parent of the first
  reviewer's directory. **Unguarded**, as the gate log above.
  (`dispatch/src/review.rs:1590-1613`)
- When a round has no reviewer, the code falls back to `checks.log` in
  the lane's working tree, where a gate's output, set values included,
  could be committed and pushed. Dispatch's own state never makes such a
  round: a stage is a review stage only when it names reviewers, every
  round is made with one reviewer per name, each under the round
  directory, and `reviewers` has no serde default. But nothing refuses a
  hand-edited ticket with an empty list, which reaches it.
  **Unguarded.**
  (`dispatch/src/review.rs:1590-1591`, `dispatch/src/review.rs:305-333`,
  `dispatch/src/pipeline.rs:705-707`, `dispatch/src/ticket.rs:625-632`)
- `checks` is a name Dispatch reads itself, so it can never be declared a
  secret artifact. **Unguarded.** (`dispatch/src/pipeline.rs:668-670`)
- `dispatch.sock` serves `checks.log` whole. **Unguarded.**
  (`dispatch/src/serve.rs:1215-1225`, a test)
- Switchboard's ticket page reads and shows it locally. **Unguarded**
  (local UI). (`src/core/dispatch.rs:1325-1341`, `src/ui/ticket.rs:251`,
  `src/ui/ticket.rs:1188`, `src/ui/ticket.rs:1232`)
- The remote port sends `checks.log` to a paired device with no
  `policy.mask`: `attempt_files` lists it as inline text, and
  `remote_ticket_file` returns the cached text, or Dispatch's fresh
  `Reply::Artifact` text, as JSON. **Unguarded**, and contrary to
  `docs/design.md:250-254`, which names redacted text as "the one other
  way a secret's surroundings leave the app".
  (`src/core/tickets.rs:221-223`, `src/core/tickets.rs:438-444`,
  `src/app.rs:2438-2497`, `src/app.rs:2344-2350`)
- Command reviewers start through `start_reviewer`, which takes no outer
  wrapper, and their child loses the runner's token, so they get no set
  values. Guarded.
  (`dispatch/src/git.rs:211-232`, `dispatch/src/git.rs:429-447`,
  `dispatch/src/git.rs:483-490`, `dispatch/src/review.rs:426-442`)
- The runner's grants serve every `env` gate it runs; a stage's names on
  a gate are not consulted. Known gap of scope, not of travel.
  (`docs/design.md:2626-2627`)

## Other readers of a resolved value

- The redaction policy holds set values in memory in `Policy.secrets` on
  the content worker, cached by spec fingerprint. Never written.
  Guarded, apart from the derived `Debug` covered in
  [`env.resolve`](#envresolve-on-controlsock-to-switchboard-env).
  (`src/content.rs:170-236`, `src/content.rs:238-245`,
  `src/content.rs:509-552`)
- Every `SecretStore::get` call outside tests: `src/app.rs:127`
  (project environment), `src/app.rs:2800` and `src/app.rs:2951-2956`
  (remote identity key), `src/app.rs:3996` (`env.resolve`),
  `src/content.rs:184` and `src/content.rs:228` (policy),
  `src/ui/env.rs:41` and `src/ui/env.rs:108-109` (env editor),
  `src/ui/prompt_box.rs:360` (voice key). Only `src/app.rs:3996` and
  `src/content.rs:184` reach a set account, both through
  `resolve_sets`. Guarded.
- The env editor covers the global and project layers only and refuses
  to save a set scope. Its rows show whether a secret is stored, not its
  value, but its preview has a Reveal button that shows resolved global
  and project secret values on screen; this does not concern sets. No
  `src/ui/` file reads `env_sets`; there is no GUI for sets. Guarded,
  for set values.
  (`src/ui/env.rs:96-115`, `src/ui/env.rs:271-297`, `src/ui/env.rs:332`,
  `docs/design.md:2614`)

## Sources

The code is cited inline at `70b2074`. Documents:

- `docs/design.md:238-259` (Trust boundary), `2540` (Secret artifacts
  gaps), `2604-2630` (Environment sets, spike 14 and gaps)
- `spikes/03-session-state/README.md:42`, `426`, `447` (the hook events,
  with no `PostToolUseFailure` payload recorded)

Checks run while writing, on throwaway tmux sockets
`switchboard-test-217-<pid>` and `switchboard-test-217b-<pid>`, tmux
3.7c, macOS 15 (Darwin 24.6.0):

- `tmux show-environment -t sb-a SWITCHBOARD_RECORD_TOKEN` and
  `... SWITCHBOARD_RECORD_ID` run from inside session `sb-b`'s pane
  printed `sb-a`'s values.
- Three malformed `new-session -e "SECRET=..."` lines piped to
  `start-server ; source-file -`: an unknown flag (`-:1: command
  new-session: unknown flag -Z`) and a duplicate session (`duplicate
  session: sb-a`) were reported without the value; an unterminated
  quote was accepted silently.
- `ps -E -o command= -p <pid>` showed a marker variable for a Homebrew
  `node` process started with it and not for `/bin/sleep`, which is a
  platform binary.

## Not found

Looked for and not found:

- A call of `resolve_sets` other than `src/app.rs:3997` and
  `src/content.rs:208`, outside tests.
- A path that puts a set value into `SpawnSpec.env`, `SessionRecord.env`
  or `settings.json`.
- A `{:?}` or `log::` of a `Request`, `Body`, `AppAction`, `Effect`,
  `SetsResolved` or `Policy` in non-test code, or of a
  `switchboard_control::Reply` in Switchboard's own non-test code.
  The `{:?}` sites in `src/app.rs` print names (1046), a uuid (1518), a
  frame kind (1716), a pairing (3667) and a `ports::dispatch::Reply`
  (4118), a different type. Dispatch's `{:?}` of a wire `Reply` is
  listed in the `env.resolve` section.
- An operations-log line for a query.
- A remote route that builds `EnvResolve`, `EnvSets` or `EnvSecretStore`.
- Request or reply logging in the wire client (`control/src/client.rs`)
  or in the socket thread (`src/adapters/control.rs:196-230`).
- An audit line naming who resolved a session's sets.
- Redaction of Dispatch ticket files (`checks.log`, findings, evidence)
  before the remote port sends them.
- A `{cmd:?}` in Dispatch that prints the runner's token or a set value
  (see [Dispatch's grants](#dispatchs-operator-and-stage-env-grants)).
- A spike recording Claude Code's `PostToolUseFailure` payload.

Not established: the three claims marked **Unverified** in
[the `session.new` launch](#the-sessionnew-launch-through-tmuxs-stdin)
(tmux parse errors),
[the launch token](#the-launch-token-which-unlocks-envresolve)
(a permission prompt for `tmux show-environment`) and
[what the child prints](#what-the-child-prints)
(`PostToolUseFailure`'s `error` field).
