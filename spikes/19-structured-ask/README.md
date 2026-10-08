# Spike 19: a card's answer reaching the pane as the next prompt

Question: when the owner answers a session's structured ask on its card,
does the line Switchboard types into the pane,
`Owner answered "<question>": <answer>`, reach interactive Claude Code
as the next user message, submitted once, both when the pane sits at
its prompt and when the answer is held through a turn until its `Stop`?

## How it was checked (2026-10-07, Claude Code 2.1.293)

The ignored gate test `a_structured_answer_arrives_as_the_next_prompt`
drives `SwitchboardApp` with the real tmux host on a private
`switchboard-test-gate-*` socket, the real hook log, and one
interactive Claude Code session with `"model": "haiku"` in a throwaway
directory under `$HOME/code_repos` (already trusted). It spends three
haiku turns:

```sh
cargo test --locked --test gate -- --ignored a_structured_answer_arrives_as_the_next_prompt --nocapture
```

The ask itself is put on the record directly rather than through
`switchboard-ask`: the app keeps only a hash of the launch token, so the
test cannot call `session.ask`. The control path is covered by the core
tests (`core::tests::answering`) and the wire and CLI unit tests.

1. Typed `reply with pong`; waited for its `Stop`.
2. **Between turns.** A choice ask `pick` with options `a`, `b`; the
   owner's `AnswerAsk` with `b`. The core cleared the ask and sent at
   once; waited for that turn's `Stop`.
3. **Mid-turn.** Typed `write four sentences about tmux, then stop`;
   once the core read the turn as open, raised the same ask (as the
   session's own `switchboard-ask` inside a turn would be) and answered
   `a`. The core kept the answer on the record and sent nothing; the
   app's next poll applied the turn's `Stop` and sent it.

## Result

```
held: Some(Ask { message: "pick", …, kind: Choice(["a", "b"]), answer: Some("a") })
user messages: ["reply with pong", "Owner answered \"pick\": b",
                "write four sentences about tmux, then stop", "Owner answered \"pick\": a"]
```

- Both answers arrived as their own user message, typed (one line, no
  `<pasted_content>` wrapper), and each was submitted once.
- The held answer went out only after the turn's `Stop`, and became the
  next user message after the typed prompt.
- The first attempt raised the ask before typing the step 3 prompt;
  that prompt, typed between turns, answered the ask and cleared it, so
  nothing was held. That is the plain-ask rule doing its job: an ask
  standing before a typed prompt is answered by it.
- Not measured: an Esc interrupt (no `Stop`, so a held answer waits;
  see `docs/design.md`, "Structured asks"), and a restart with an answer
  pending, which the core test
  `an_answer_left_by_a_restart_goes_out_on_the_first_host_list` covers.
