#!/bin/sh
# One pipeline stage in isolation; see the doc comment in stage.py.
# usage: stage.sh <investigate|facts|plan|review|respond> <label> <model> <a|b|...> <from results dir> [pipeline.toml]
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exec uv run -q --with tomli "$here/stage.py" "$@"
