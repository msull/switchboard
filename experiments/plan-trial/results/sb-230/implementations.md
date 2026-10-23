# #230 implemented four times

Two finalized plans, each revised once with the owner's two calls (no
certificate reissue, cookie named with the server id) through the
pipeline's own owner-edit step, each built by a sonnet and a haiku
implementer, each branch then read by the pipeline's correctness and
style reviewers (sonnet on every branch, so the reviews compare) and
fixed by its own arm's model. Every branch passes the implement gate
(fmt, clippy with warnings denied, the whole test suite) after its fix
round. Nothing was run in a browser: the pages are tested on the host
side only, plus node tests of the pure functions where the plan asked
for them. The owner's phone is the test that remains.

| plan | implementer | implement $ / min | insertions | correctness / style points | fix $ / min | final insertions | gate |
|---|---|---|---|---|---|---|---|
| fable-b revised | sonnet | 2.36 / 20 | 1258 | 3 / 8 | 0.34 / 10 | 1282 | green |
| lean-b revised | sonnet | 2.33 / 18 | 1606 | 3 / 9 | 0.65 + 0.23 / 12 | 1661 | green (second run; a dispatch test flaked under load) |
| fable-b revised | haiku | 2.06 / 21 | 1404 | 4 / 7 | 0.10 / 7 | 1428 | green |
| lean-b revised | haiku | 2.04 / 25 | 2088 | 3 / 8 | 0.13 / 10 | 2177 | green |

Reviews cost $0.24 to $0.42 each at sonnet. The revise step cost
$0.24 (fable-b) and $0.38 (lean-b). Two runs were lost to a full disk
and kept as `-attempt1` directories: both haiku implementers (their
uncommitted edits in `partial.diff`) and the first lean-b sonnet fix,
whose second attempt continued on the same worktree.

## Where the branches are

Worktrees under `~/code_repos/plan-trial/`, branches of the clone
`~/code_repos/plan-trial/switchboard`, all on base 7fe8949:

- `sb-230-stage-implement-v5rev-sonnet-fableb`, branch `stage/sb-230-implement-v5rev-sonnet-fableb` (abe87c6 + fixup d104728)
- `sb-230-stage-implement-v5rev-sonnet-leanb`, branch `stage/sb-230-implement-v5rev-sonnet-leanb` (47f139e + fixup aabfb7e)
- `sb-230-stage-implement-v5rev-haiku-fableb`, branch `stage/sb-230-implement-v5rev-haiku-fableb` (dc1c58a + fixup 7a22de4)
- `sb-230-stage-implement-v5rev-haiku-leanb`, branch `stage/sb-230-implement-v5rev-haiku-leanb` (f693952 + fixup 13eabd0)

Each run's prompt, argv, transcript, notes or response, diff, commits
and gate output are under `stages/implement`, `stages/review-code`
and `stages/fix`. Nothing is pushed.

## What the reviewers found

The correctness points on all four branches are in the page script:
a refresh dropped while one is running, the open session's text never
refreshed by the poll, a redraw wiping the reply box, a 401 after
pairing not shown as revoked, the socket retried every poll, a crash
when the user leaves a loading session. One point on each sonnet branch
and one on a haiku branch is in the Mac's dialog (loopback shown with
no notice when no address is found). None is in the host's Rust: the
host side had tests the plan asked for and the page did not. The style
points are the usual: logic in the UI function, a prefix defined twice,
a wrapper with one caller, doc rows not updated, a lint silenced.

## Reading

- Sonnet and haiku produced the same shape of branch from the same
  plan at nearly the same price and time; haiku's branches are longer
  (1404 and 2088 insertions against 1258 and 1606) and took more turns.
  The reviewers found three or four correctness points on each, of the
  same kind. On this evidence the implementer's model mattered less
  than the plan's, and the plan's less than the fact that nobody ran a
  browser.
- The lean-b plan, which asks for a node test file and a spike README,
  produced branches a third larger than fable-b's, which asks for
  neither.
- A fix round at haiku costs a tenth of one at sonnet and addressed
  every point the reviewers raised, by the fixer's own account; whether
  it addressed them well is the next review round's question, not run.
