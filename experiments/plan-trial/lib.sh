# Shared by the trial scripts. Sourced, not run.
# The dispatch CLI: the one on PATH, else the one beside the app's binary.
if command -v dispatch >/dev/null 2>&1; then
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
