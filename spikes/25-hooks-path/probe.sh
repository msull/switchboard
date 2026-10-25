#!/bin/sh
# Which git commands run which hook, with core.hooksPath=.githooks, and does
# `-c core.hooksPath=<missing dir>` silence them. Throwaway repos under a temp dir.
set -u
T=$(mktemp -d); cd "$T"
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
git init -q -b main origin && cd origin
mkdir .githooks
for h in pre-commit pre-rebase pre-push post-checkout reference-transaction post-rewrite; do
  printf '#!/bin/sh\necho "%s $PWD" >> "%s/hooklog"\n' "$h" "$T" > .githooks/$h; chmod +x .githooks/$h
done
echo 1 > f; git add -A; git commit -qm one; echo 2 > f; git commit -qam two
cd "$T"; git clone -q origin clone; cd clone
git config user.name a; git config user.email a@x
log() { echo "--- $1"; sort "$T/hooklog" 2>/dev/null | uniq -c | sed "s|$T|T|g"; rm -f "$T/hooklog"; }
git config core.hooksPath .githooks
log "set config (nothing expected)"
git worktree add -q ../wt -b lane main; log "worktree add, with hooksPath"
cd ../wt
echo 3 > g; git add g; git commit -qm three; log "commit in linked worktree (cwd = worktree top)"
git commit -q --amend --no-edit; log "commit --amend"
mkdir sub; (cd sub && echo 4 > h && git add h && git commit -qm four); log "commit from a subdirectory"
git commit-tree -p HEAD -m x HEAD^{tree} >/dev/null; log "commit-tree"
git merge-tree --write-tree HEAD main >/dev/null; log "merge-tree --write-tree"
git update-ref refs/heads/zz HEAD; log "update-ref"
git checkout -q --detach; log "checkout --detach"
git checkout -q lane; git rebase -q main; log "rebase main (lane has 2 picks)"
git -c core.hooksPath=/nonexistent/x commit -q --allow-empty -m e; log "commit with -c core.hooksPath=/nonexistent/x"
git -c core.hooksPath=/nonexistent/x rebase -q --force main; log "rebase with -c missing"
git -c core.hooksPath=/nonexistent/x checkout -q --detach; git -c core.hooksPath=/nonexistent/x update-ref refs/heads/yy HEAD; log "checkout+update-ref with -c missing"
git -c core.hooksPath=/nonexistent/x worktree add -q ../wt2 -b lane2 main; log "worktree add with -c missing"
git -c core.hooksPath=/nonexistent/x push -q --force-with-lease origin lane:lane 2>&1; log "push with -c missing"
git push -q origin lane:lane2 2>&1; log "push with hooksPath (pre-push)"
cd "$T"; git config -f clone/.git/config --unset core.hooksPath
git -C clone config core.hooksPath .nothere; git -C clone worktree add -q ../wt3 -b l3 main; echo 5 > wt3/z; git -C wt3 add z; git -C wt3 commit -qm z; log "hooksPath naming a missing relative dir (no error?)"
git -c core.hooksPath=.githooks clone -q origin c2; log "clone with -c hooksPath=.githooks (post-checkout runs in clone?)"
rm -rf "$T"
