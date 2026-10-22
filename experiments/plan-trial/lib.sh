# Shared by the trial scripts. Sourced, not run.
# The dispatch CLI: one built from this branch when there is one (it
# takes an issue once per project, which the 0.4.0 release does not),
# else the one on PATH, else the one beside the app's binary.
here_lib=$(cd "$(dirname "$0")" && pwd)
if [ -x "$here_lib/../../target/release/dispatch" ]; then
  DISPATCH="$here_lib/../../target/release/dispatch"
elif command -v dispatch >/dev/null 2>&1; then
  DISPATCH=dispatch
else
  DISPATCH="$HOME/Applications/Switchboard.app/Contents/MacOS/dispatch"
fi
DATA=${DISPATCH_DATA_DIR:-"$HOME/Library/Application Support/Dispatch"}
# repo_vars <switchboard|simplesingletable>: sets short and gh_repo.
repo_vars() {
  case $1 in
    switchboard) short=sb; gh_repo=msull/switchboard ;;
    simplesingletable) short=sst; gh_repo=msull/simplesingletable ;;
    *) echo "unknown repository $1 (switchboard or simplesingletable)" >&2; exit 2 ;;
  esac
}
# The facts pipeline's scenarios: a name and the model of each of its
# four planning stages, in the order investigate facts plan review.
# `mixed` spends on the two stages that gather and check information
# and keeps the two that write on sonnet.
FACTS_SCENARIOS="sonnet opus mixed"
facts_models() {
  case $1 in
    sonnet) echo "sonnet sonnet sonnet sonnet" ;;
    opus)   echo "opus opus opus opus" ;;
    mixed)  echo "sonnet opus sonnet opus" ;;
    *) echo "unknown scenario $1 ($FACTS_SCENARIOS)" >&2; exit 2 ;;
  esac
}
