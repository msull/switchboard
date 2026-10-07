# Spike 16: the base pipeline on a merge commit

Question: once a lane's pull request merges, can Dispatch find the
commit it merged as and read the base's pipeline on that commit, so a
lane that merges after it (`merge_after`, `merge_after_deploy`) waits
for the deploy rather than for the merge alone?

## GitHub (run on 2026-10-07 against this repository)

The merge commit is on both reads the runner makes of a pull request.
`gh pr list` accepts `mergeCommit` in `--json` and fills it for a
merged row; `gh pr view` does the same. An open pull request has
`"mergeCommit": null`.

```sh
gh pr list --repo msull/switchboard --head <branch> --state all --json number,state,mergeCommit
# [{"mergeCommit":{"oid":"054d02d75414f015265e3f5b27a8039d676bb497"},"number":145,"state":"MERGED"}]
gh pr view 145 --repo msull/switchboard --json mergeCommit,state
# {"mergeCommit":{"oid":"054d02d75414f015265e3f5b27a8039d676bb497"},"state":"MERGED"}
```

The check runs on the merge commit are the base's run. Read a few
minutes after the merge, they were still running:

```sh
gh api repos/msull/switchboard/commits/054d02d75414f015265e3f5b27a8039d676bb497/check-runs \
  --jq '{total_count, runs: [.check_runs[] | {name, status, conclusion}]}'
# {"runs":[{"conclusion":null,"name":"msrv","status":"in_progress"},
#          {"conclusion":null,"name":"check","status":"in_progress"}],"total_count":2}
```

and on an older merge commit, finished:

```
[{"conclusion":"success","name":"check","status":"completed"},
 {"conclusion":"success","name":"msrv","status":"completed"}]
```

A short hash is accepted (`commits/054d02d/check-runs` gave the same
two runs). The combined `commits/<sha>/status` endpoint read
`{"state":"pending","total_count":0}` for the same commit: it carries
only legacy statuses, not check runs, so it is not what is read.

`Gh::commit_run` therefore reads `check-runs?per_page=100`: no runs is
`none`, any run not `completed` is `pending`, a completed run whose
conclusion is not `success`, `neutral` or `skipped` is failed by name,
and otherwise `passed`. With a step named, only the runs of that name
count.

## Bitbucket (not run)

The reads against a merged pull request of the client project were not
made: the run's permission rules refused a script that read the
Bitbucket token from `<data>/env`. What is built rests on Bitbucket
Cloud's documented shapes, and is written so that a wrong guess
degrades to a "but" question rather than a wrong merge:

- The pull request object has `merge_commit: {"hash": "<12 hex>"}` once
  merged, on both the list (`pullrequests?q=source.branch.name=...`)
  and the single read (`pullrequests/{id}`). `PrRow` parses it from
  both. If the list rows lack it, the runner reads the pull request by
  number once, on the reading that saw it merged
  (`merge_commit_by_number`); if neither has it, the waiting lane is
  asked with "its merge commit was not reported, so its base pipeline
  was not read".
- `GET /repositories/{repo}/commit/{hash}/statuses?pagelen=100`, with
  the short hash, read through `parse_statuses` like a pull request's
  statuses: `INPROGRESS` is pending, `FAILED` or `STOPPED` failed by
  name, all `SUCCESSFUL` passed, none is `none`.
- For a named step: `GET /repositories/{repo}/pipelines/?sort=-created_on&pagelen=50`,
  the newest pipeline whose `target.commit.hash` (full) starts with the
  short hash, then `GET .../pipelines/{uuid}/steps/?pagelen=100` and
  that step's `state.name` (`COMPLETED` or not) and
  `state.result.name` (`SUCCESSFUL` or not). A filter on
  `target.commit.hash` in the query was not relied on, since it is not
  documented.

Still to check by hand, with `curl` and the account token, against a
merged pull request of a repository whose base runs a deploy:

1. Does each list row carry `merge_commit.hash`, and is it the commit
   the base's pipeline ran on (not a squash's parent)?
2. While a later step runs, does the commit's status read
   `INPROGRESS`? Does a manual step that is never triggered leave it
   `INPROGRESS` (the wait then asks with "has not finished after 60
   minutes")? Does a rerun on the same commit turn it back?
3. Is the newest pipeline on the commit among the first 50 by
   `-created_on` on a busy repository?

Until these are run, nothing waits on a Bitbucket base pipeline:
validation refuses `merge_after_deploy` where the pipeline file names
Bitbucket, and a Bitbucket pull request met at run time ends the wait
with a "but" without calling `commit_run`. A `merge_after` without it
waits for the merge alone, which reads only the pull request's state.

Shapes recorded in parse tests are scrubbed: the workspace and
repository are `example-co/orchard-backend`, branches `feature/x`,
steps `Build`, `Deploy to dev`, and hashes and UUIDs made up at the
real lengths (12 hex for a pull request's hashes, 40 for a pipeline's
target, a braced UUID for a pipeline).
