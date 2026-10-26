#!/bin/sh
# A rebase that really replays commits: which hooks run, with and without -c missing.
set -u
T=$(mktemp -d); cd "$T"
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
git init -q -b main r && cd r && mkdir .githooks
for h in pre-commit pre-rebase post-checkout post-rewrite; do
  printf '#!/bin/sh\necho "%s" >> "%s/hooklog"\n' "$h" "$T" > .githooks/$h; chmod +x .githooks/$h; done
echo 1 > f; git add -A; git commit -qm one
git checkout -qb lane; echo a > a; git add a; git commit -qm a; echo b > b; git add b; git commit -qm b
git checkout -q main; echo 2 > f; git commit -qam two
git config core.hooksPath .githooks; rm -f "$T/hooklog"
git checkout -q lane; rm -f "$T/hooklog"
git rebase -q main; echo "--- rebase with hooksPath"; sort "$T/hooklog" | uniq -c; rm -f "$T/hooklog"
git reset -q --hard HEAD~2; git cherry-pick -q lane@{1} 2>/dev/null
git -c core.hooksPath=/nonexistent/x rebase -q main 2>&1; echo "--- rebase with -c missing"; cat "$T/hooklog" 2>/dev/null
rm -rf "$T"
