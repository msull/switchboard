"""One pipeline stage in isolation, fed what the pipeline would feed it.

usage: uv run --with tomli stage.py <stage> <label> <model> <rep> <from> [pipeline.toml]

<stage>   investigate | facts | plan | review | respond | review2 | respond2 | revise | implement | review-code | fix
<label>   a word for this guidance revision, so runs group: v1, v2-evidence
<from>    a results directory of a finished run (results/sb-230/facts/opus-a)
          whose files are the stage's inputs: notes.md for facts, notes.md
          and facts.md for plan, plan.first.md and facts.md for review,
          plan.first.md and plan.feedback-1.md for respond; review2 takes
          the respond stage's plan.md and plan.response-1.md (PLAN and
          RESPONSE) and writes plan.feedback-2.md, respond2 answers it;
          revise is the owner's edit (TEXT holds the words, PLAN the plan);
          implement takes a final plan (PLAN) and facts and builds on the
          worktree, then records the commits, the diff and the gate's
          result (gate.txt); review-code runs one of that stage's reviewers
          (REVIEWER=correctness or style) on an implement run's branch,
          with TREE naming the implement run's worktree; fix runs the
          fixer on that worktree over the feedback files FEEDBACK names
          (colon-separated) and records the commits and the gate again. An
          environment variable NOTES, FACTS, PLAN, FEEDBACK or RESPONSE
          naming a file overrides
          the one from there, so a stage can be fed the previous stage's
          harness output.
pipeline  the file whose guidance and templates to use; default
          pipelines/switchboard-facts.toml.

What Dispatch hands a stage session, reproduced here: the operator's
guidance, a blank line, then the stage prompt with its fields filled;
the operator's args as flags, with the model replaced; an Edit rule for
the directory the artifacts go in; the lane worktree as cwd; the input
paths as DISPATCH_INPUT_<NAME>. A review round gets only its template:
Dispatch renders the review's {worktree}, {branch} and {inputs.*} and
Switchboard fills {plan}, {feedback}, {response}, {no_feedback} per
round, and the reviewer's guidance is not sent at all. Two things
differ: the session is `claude -p` rather than an interactive pane, and
it runs with --permission-mode acceptEdits in place of whatever the
trial Mac's own settings allow. `respond` is an approximation: the
pipeline resumes the planner's session, which cannot be done here, so a
fresh session gets the planner's guidance and the respond template.

Writes results/<issue dir>/stages/<stage>/<label>-<model>-<rep>/ with
prompt.md and argv.txt (diff them against a captured session), the
artifacts, run.json, the session transcript, seconds.txt, the pipeline
file used, and for facts the files committed on the run's branch. With
STAGE_DRY=1 in the environment it writes the prompt and argv and stops.
"""
import json, os, pathlib, shutil, subprocess, sys, time

try:
    import tomllib
except ModuleNotFoundError:
    import tomli as tomllib

HERE = pathlib.Path(__file__).resolve().parent
ROOT = pathlib.Path(os.environ.get("PLAN_TRIAL_ROOT", pathlib.Path.home() / "code_repos/plan-trial"))
REPO = "msull/switchboard"
NO_FEEDBACK = "No further feedback."


def die(msg):
    sys.exit(f"stage.py: {msg}")


def run(argv, **kw):
    return subprocess.run(argv, check=True, text=True, capture_output=True, **kw).stdout


def issue_json(res):
    f = res / "issue.json"
    if not f.exists():
        number = res.name.split("-", 1)[1]
        f.write_text(run(["gh", "issue", "view", number, "--repo", REPO, "--json", "number,title,body"]))
    return json.loads(f.read_text())


def render(text, vars):
    for k, v in vars.items():
        text = text.replace("{" + k + "}", str(v))
    return text


IMPLEMENT_ALLOW = [
    "Bash(cargo:*)", "Bash(rustfmt:*)", "Bash(swift:*)",
    "Bash(git add:*)", "Bash(git commit:*)", "Bash(git status:*)", "Bash(git diff:*)",
    "Bash(git log:*)", "Bash(git show:*)", "Bash(git rm:*)", "Bash(git mv:*)",
    "Bash(git restore:*)", "Bash(git rev-parse:*)", "Bash(git grep:*)", "Bash(git ls-files:*)",
    "Bash(git stash:*)", "Bash(git blame:*)",
    "Bash(ls:*)", "Bash(cat:*)", "Bash(head:*)", "Bash(tail:*)", "Bash(wc:*)", "Bash(grep:*)",
    "Bash(rg:*)", "Bash(find:*)", "Bash(sed -n:*)", "Bash(mkdir:*)", "Bash(diff:*)",
]

REVIEW_ALLOW = [r for r in IMPLEMENT_ALLOW if not any(
    r.startswith(f"Bash(git {w}") for w in ("add", "commit", "rm", "mv", "restore", "stash"))]

def worktree(res, name):
    """A writable worktree at the issue's base commit, on its own branch."""
    base = (res / "base.txt").read_text().strip()
    clone = ROOT / "switchboard"
    tree = ROOT / f"{res.name}-stage-{name}"
    ROOT.mkdir(parents=True, exist_ok=True)
    if not (clone / ".git").exists():
        run(["git", "clone", "-q", f"https://github.com/{REPO}.git", str(clone)])
    if not tree.exists():
        try:
            run(["git", "-C", str(clone), "fetch", "-q", "origin", base])
        except subprocess.CalledProcessError:
            run(["git", "-C", str(clone), "fetch", "-q", "origin"])
        # The branch carries the issue too: two issues share one clone.
        run(["git", "-C", str(clone), "worktree", "add", "-q", "-b", f"stage/{res.name}-{name}", str(tree), base])
        # The lane setup the pipeline runs: a target directory of its own,
        # excluded from status through the clone's info/exclude.
        (tree / ".cargo").mkdir(exist_ok=True)
        (tree / ".cargo/config.toml").write_text('[build]\ntarget-dir = "target"\n')
        exclude = pathlib.Path(run(["git", "-C", str(tree), "rev-parse", "--git-common-dir"]).strip()) / "info/exclude"
        if not exclude.is_absolute():
            exclude = tree / exclude
        lines = exclude.read_text().splitlines() if exclude.exists() else []
        if ".cargo/config.toml" not in lines:
            exclude.parent.mkdir(parents=True, exist_ok=True)
            exclude.write_text("\n".join(lines + [".cargo/config.toml"]) + "\n")
    return tree, base


def main():
    if len(sys.argv) < 6:
        die(__doc__)
    stage, label, model, rep, from_dir = sys.argv[1:6]
    pipeline = pathlib.Path(sys.argv[6]) if len(sys.argv) > 6 else HERE / "pipelines/switchboard-facts.toml"
    from_dir = pathlib.Path(from_dir).resolve()
    # The issue directory: the nearest ancestor holding base.txt, so a
    # stage run's own directory (results/<issue>/stages/<stage>/<name>)
    # serves as <from> as well as an arm's.
    res = next((d for d in from_dir.parents if (d / "base.txt").exists()), None)
    if res is None:
        die(f"{from_dir} is not under a results/<issue> directory with base.txt")
    p = tomllib.loads(pipeline.read_text())
    name = f"{label}-{model}-{rep}"
    out = res / "stages" / stage / name
    if out.exists():
        die(f"{out} exists; pick another label or rep")
    out.mkdir(parents=True)
    (out / "inputs").mkdir()
    shutil.copy(pipeline, out / "pipeline.toml")
    (out / "from.txt").write_text(f"{from_dir}\n")
    issue = issue_json(res)
    if os.environ.get("TREE"):
        tree = pathlib.Path(os.environ["TREE"]).resolve()
        base = (res / "base.txt").read_text().strip()
    else:
        tree, base = worktree(res, f"{stage}-{name}")
    branch = run(["git", "-C", str(tree), "rev-parse", "--abbrev-ref", "HEAD"]).strip()
    vars = {"issue.number": issue["number"], "issue.title": issue["title"], "issue.body": issue["body"],
            "worktree": tree, "branch": branch, "no_feedback": NO_FEEDBACK}
    env = dict(os.environ)

    def take(kind, default_name, var_name):
        """An input file: the override, else the one from the finished run."""
        src = pathlib.Path(os.environ[var_name]) if os.environ.get(var_name) else from_dir / default_name
        if not src.exists():
            die(f"input {kind}: {src} is missing")
        dst = out / "inputs" / f"{kind}.md"
        shutil.copy(src, dst)
        return dst

    stages = {s["name"]: s for s in p["stages"]}
    artifacts = []
    if stage in ("investigate", "facts", "plan"):
        st = stages[stage]
        op = p["operators"][st["operator"]]
        if stage == "facts":
            notes = take("notes", "notes.md", "NOTES")
            vars["inputs.notes"] = notes
            env["DISPATCH_INPUT_NOTES"] = str(notes)
        if stage == "plan":
            notes = take("notes", "notes.md", "NOTES")
            facts = take("facts", "facts.md", "FACTS")
            vars["inputs.notes"] = notes
            vars["inputs.facts"] = facts
            env["DISPATCH_INPUT_NOTES"] = str(notes)
            env["DISPATCH_INPUT_FACTS"] = str(facts)
        for w in st.get("writes", []):
            vars[w] = out / f"{w}.md"
            artifacts.append(out / f"{w}.md")
        guidance = render(op.get("guidance", "").strip(), vars)
        prompt = (guidance + "\n\n" if guidance else "") + render(st["prompt"], vars)
        args = list(op.get("args", []))
    elif stage == "review":
        st = stages["review"]
        op = p["operators"][st["review"]]
        rv = op["review"]
        plan = take("plan", "plan.first.md", "PLAN")
        facts = take("facts", "facts.md", "FACTS")
        vars["inputs.facts"] = facts
        vars["plan"] = plan
        vars["feedback"] = out / "plan.feedback-1.md"
        artifacts.append(out / "plan.feedback-1.md")
        template = rv["review_first"]
        prompt = render(template, vars)
        if "{worktree}" not in template and "{project.root}" not in template:
            prompt = f"The repository this is about is at {tree}; paths like CLAUDE.md are relative to it. {prompt}"
        args = list(op.get("args", []))
    elif stage == "review2":
        # The second round: the reviewer gets the planner's answer and
        # the revised plan, and writes feedback 2. PLAN names the plan
        # the respond stage wrote, RESPONSE its answer.
        st = stages["review"]
        op = p["operators"][st["review"]]
        rv = op["review"]
        plan = take("plan", "plan.md", "PLAN")
        response = take("response", "plan.response-1.md", "RESPONSE")
        facts = take("facts", "facts.md", "FACTS")
        vars["inputs.facts"] = facts
        vars["plan"] = plan
        vars["response"] = response
        vars["feedback"] = out / "plan.feedback-2.md"
        artifacts.append(out / "plan.feedback-2.md")
        template = rv["review_round"]
        prompt = render(template, vars)
        if "{worktree}" not in template and "{project.root}" not in template:
            prompt = f"The repository this is about is at {tree}; paths like CLAUDE.md are relative to it. {prompt}"
        args = list(op.get("args", []))
    elif stage in ("respond", "respond2"):
        n = "2" if stage == "respond2" else "1"
        st = stages["review"]
        rv = p["operators"][st["review"]]["review"]
        op = p["operators"][stages["plan"]["operator"]]
        plan_in = take("plan", "plan.first.md" if n == "1" else "plan.md", "PLAN")
        feedback = take("feedback", f"plan.feedback-{n}.md", "FEEDBACK")
        plan = out / "plan.md"
        shutil.copy(plan_in, plan)
        vars["plan"] = plan
        vars["feedback"] = feedback
        vars["response"] = out / f"plan.response-{n}.md"
        artifacts += [plan, out / f"plan.response-{n}.md"]
        guidance = render(op.get("guidance", "").strip(), vars)
        prompt = (guidance + "\n\n" if guidance else "") + render(rv["respond"], vars)
        args = list(op.get("args", []))
    elif stage == "revise":
        # The owner's edit between rounds: the planner gets its guidance
        # and the respond_to_user template with TEXT as the owner's words.
        st = stages["review"]
        rv = p["operators"][st["review"]]["review"]
        op = p["operators"][stages["plan"]["operator"]]
        text = os.environ.get("TEXT") or die("revise needs TEXT, the owner's words")
        plan_in = take("plan", "plan.md", "PLAN")
        plan = out / "plan.md"
        shutil.copy(plan_in, plan)
        (out / "text.md").write_text(text + "\n")
        vars["plan"] = plan
        vars["text"] = text
        vars["response"] = out / "plan.response-owner.md"
        artifacts += [plan, out / "plan.response-owner.md"]
        guidance = render(op.get("guidance", "").strip(), vars)
        prompt = (guidance + "\n\n" if guidance else "") + render(rv["respond_to_user"], vars)
        args = list(op.get("args", []))
    elif stage == "implement":
        # The implement stage on a finalized plan. On the trial Mac the
        # implementer's shell commands go through that Mac's own
        # permission settings and a pane someone can answer; here they
        # are allowed by rule for the build, the tests and the local git
        # commands the guidance asks for, and nothing that reaches a
        # remote. A command outside the list is refused silently in
        # claude -p and the transcript shows it.
        st = stages["implement"]
        op = p["operators"][st["operator"]]
        plan = take("plan", "plan.md", "PLAN")
        facts = take("facts", "facts.md", "FACTS")
        vars["inputs.plan"] = plan
        vars["inputs.facts"] = facts
        env["DISPATCH_INPUT_PLAN"] = str(plan)
        env["DISPATCH_INPUT_FACTS"] = str(facts)
        vars["notes"] = out / "notes.md"
        artifacts.append(out / "notes.md")
        guidance = render(op.get("guidance", "").strip(), vars)
        prompt = (guidance + "\n\n" if guidance else "") + render(st["prompt"], vars)
        args = list(op.get("args", []))
        for rule in IMPLEMENT_ALLOW:
            args += ["--allowedTools", rule]
    elif stage == "review-code":
        # One of the review-code stage's reviewers on an implemented
        # branch. In the pipeline the reviewer runs under Switchboard's
        # code-review workflow, whose templates live in the app; here it
        # gets the operator's guidance and one paragraph naming the
        # branch, the plan and where to write, which is the approximation.
        # <from> is the implement run's directory: its worktree holds the
        # branch, its inputs the plan and facts. REVIEWER picks the
        # operator (correctness by default, or style).
        who = os.environ.get("REVIEWER", "correctness")
        op = p["operators"][who]
        plan = take("plan", "inputs/plan.md", "PLAN")
        facts = take("facts", "inputs/facts.md", "FACTS")
        vars["plan"] = plan
        vars["inputs.facts"] = facts
        vars["feedback"] = out / f"feedback.{who}.md"
        artifacts.append(out / f"feedback.{who}.md")
        guidance = render(op.get("guidance", "").strip(), vars)
        prompt = (guidance + "\n\n" if guidance else "") + (
            f"Review branch {branch} in the worktree at {tree}: the change is every commit since {base} "
            f"(git log and git diff {base}..HEAD). It implements the plan at {plan}; the facts the plan rests on are at {facts}. "
            f"Write your feedback to {vars['feedback']}.")
        args = list(op.get("args", []))
        for rule in REVIEW_ALLOW:
            args += ["--allowedTools", rule]
    elif stage == "fix":
        # The fixer on one round of code review: FEEDBACK names the
        # feedback files, separated by colons, and TREE the implement
        # run's worktree. As with review-code, the prompt paragraph
        # stands in for the workflow template the app holds.
        op = p["operators"][stages["review-code"]["implementer"]]
        plan = take("plan", "inputs/plan.md", "PLAN")
        vars["plan"] = plan
        fbs = []
        for i, f in enumerate(os.environ.get("FEEDBACK", "").split(":")):
            if not f:
                die("fix needs FEEDBACK, the feedback files separated by colons")
            dst = out / "inputs" / f"feedback-{i + 1}.md"
            shutil.copy(f, dst)
            fbs.append(dst)
        vars["response"] = out / "response.md"
        artifacts.append(out / "response.md")
        guidance = render(op.get("guidance", "").strip(), vars)
        prompt = (guidance + "\n\n" if guidance else "") + (
            f"The branch {branch} in the worktree at {tree} (every commit since {base}) implements the plan at {plan}. "
            f"The review round's points are at {' and '.join(str(f) for f in fbs)}. "
            f"Address them on the branch, commit, and write your response, one entry per point, to {vars['response']}.")
        args = list(op.get("args", []))
        for rule in IMPLEMENT_ALLOW:
            args += ["--allowedTools", rule]
    else:
        die(f"unknown stage {stage}")

    # The operator's args with the model swapped, then Dispatch's edit rule.
    flags = []
    i = 0
    while i < len(args):
        if args[i] == "--model":
            i += 2
            continue
        flags.append(args[i])
        i += 1
    argv = ["claude", "-p", "--model", model, "--output-format", "json", "--permission-mode", "acceptEdits"]
    argv += flags
    argv += ["--allowedTools", f"Edit(//{out}/**)"]
    (out / "prompt.md").write_text(prompt)
    (out / "argv.txt").write_text("\n".join(argv) + f"\ncwd {tree}\n" + "".join(
        f"{k}={v}\n" for k, v in env.items() if k.startswith("DISPATCH_INPUT_")))
    (out / "worktree.txt").write_text(f"{tree}\n")
    print(f"{stage} {name}: prompt {len(prompt.split())} words, cwd {tree}")
    if os.environ.get("STAGE_DRY"):
        print(f"dry run: prompt.md and argv.txt written under {out}; nothing launched")
        return
    start = time.time()
    with open(out / "prompt.md") as fin, open(out / "run.json", "w") as fout:
        rc = subprocess.run(argv, stdin=fin, stdout=fout, cwd=tree, env=env).returncode
    (out / "seconds.txt").write_text(f"{int(time.time() - start)}\n")
    if rc:
        print(f"claude exited {rc}", file=sys.stderr)
    try:
        r = json.loads((out / "run.json").read_text())
        r = r[-1] if isinstance(r, list) else r
        print(f"cost_usd={r.get('total_cost_usd')} turns={r.get('num_turns')} ms={r.get('duration_ms')}")
        # The session's transcript, for the write-up: what it searched,
        # read and wrote, turn by turn. Claude Code keeps it under
        # ~/.claude/projects/<cwd slug>/<session id>.jsonl.
        sid = r.get("session_id")
        if sid:
            for t in (pathlib.Path.home() / ".claude/projects").glob(f"*/{sid}.jsonl"):
                shutil.copy(t, out / "transcript.jsonl")
    except (ValueError, OSError):
        print("run.json unreadable", file=sys.stderr)
    for a in artifacts:
        print(f"{a}: {'written, ' + str(len(a.read_text().split())) + ' words' if a.exists() else 'MISSING'}")
    if stage in ("implement", "fix"):
        g = lambda *a: subprocess.run(["git", "-C", str(tree), *a], text=True, capture_output=True).stdout
        (out / "commits.txt").write_text(g("log", "--format=%h %s", f"{base}..HEAD"))
        (out / "diffstat.txt").write_text(g("diff", "--stat", base, "HEAD"))
        (out / "branch.diff").write_text(g("diff", base, "HEAD"))
        dirty = g("status", "--porcelain")
        (out / "dirty.txt").write_text(dirty)
        print(f"commits:\n{(out / 'commits.txt').read_text()}dirty: {'yes' if dirty.strip() else 'no'}")
        # The stage's gate, as the pipeline would run it in the lane.
        gate = stages["implement"]["gate"]["argv"]
        t0 = time.time()
        res_g = subprocess.run(gate, cwd=tree, text=True, capture_output=True)
        (out / "gate.txt").write_text(f"exit {res_g.returncode} in {int(time.time() - t0)} s\n\n" + res_g.stdout[-20000:] + res_g.stderr[-20000:])
        print(f"gate: exit {res_g.returncode} in {int(time.time() - t0)} s")
    if stage == "facts":
        changed = run(["git", "-C", str(tree), "diff", "--name-only", base, "HEAD"])
        dirty = run(["git", "-C", str(tree), "status", "--porcelain"])
        (out / "spikes.txt").write_text(changed)
        if changed.strip():
            (out / "spike.diff").write_text(run(["git", "-C", str(tree), "diff", base, "HEAD"]))
            print(f"spike: {len(changed.split())} files committed")
        if dirty.strip():
            (out / "dirty.txt").write_text(dirty)
            print("tree left dirty (the pipeline's gate would have failed):\n" + dirty, file=sys.stderr)


if __name__ == "__main__":
    main()
