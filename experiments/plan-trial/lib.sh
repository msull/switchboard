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
# and keeps the two that write on sonnet; `cheap` is the same shape one
# tier down, haiku writing and sonnet checking, with no opus anywhere.
FACTS_SCENARIOS="sonnet opus mixed haiku cheap"
facts_models() {
  case $1 in
    sonnet) echo "sonnet sonnet sonnet sonnet" ;;
    opus)   echo "opus opus opus opus" ;;
    mixed)  echo "sonnet opus sonnet opus" ;;
    haiku)  echo "haiku haiku haiku haiku" ;;
    cheap)  echo "haiku sonnet haiku sonnet" ;;
    *) echo "unknown scenario $1 ($FACTS_SCENARIOS)" >&2; exit 2 ;;
  esac
}

# The reassembled pipeline (revision 5) as a second set of arms, prefix
# sbf5, results under facts5/. The three combinations the stage-level
# runs argued for: lean (haiku reads, sonnet checks and writes), sonnet
# throughout, and checked (sonnet with the opus reviewer, which searched
# in every isolated run where sonnet did not).
# opus and fable throughout are the owner's two additions: the ceiling,
# and a look at how the strongest model handles each stage.
FACTS5_SCENARIOS="lean sonnet checked opus fable"
facts5_models() {
  case $1 in
    lean)    echo "haiku sonnet sonnet sonnet" ;;
    sonnet)  echo "sonnet sonnet sonnet sonnet" ;;
    checked) echo "sonnet sonnet sonnet opus" ;;
    opus)    echo "opus opus opus opus" ;;
    fable)   echo "fable fable fable fable" ;;
    *) echo "unknown scenario $1 ($FACTS5_SCENARIOS)" >&2; exit 2 ;;
  esac
}
# set_vars <facts|facts5>: the arm prefix, the pipeline file, the
# scenarios and their model function, and the results subdirectory.
set_vars() {
  case $1 in
    facts)  prefix=sbf;  pipeline=switchboard-facts.toml;    scenarios=$FACTS_SCENARIOS;  models=facts_models;  subdir=facts ;;
    facts5) prefix=sbf5; pipeline=switchboard-facts-v5.toml; scenarios=$FACTS5_SCENARIOS; models=facts5_models; subdir=facts5 ;;
    *) echo "unknown set $1 (facts or facts5)" >&2; exit 2 ;;
  esac
}
