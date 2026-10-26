#!/bin/bash
# CI's apt step: the three packages the build links, with every apt call
# bounded by wall clock and retried. apt's own acquire timeout does not
# end a stall where the Azure mirror holds the connection open and sends
# nothing (one `update` sat behind it for 86 minutes), so the bound is
# `timeout` around the whole call; the next call usually lands on a
# different mirror node. A healthy run takes about ten seconds.
set -euo pipefail

packages=(libxkbcommon-dev libasound2-dev cmake)
opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=10)

bounded() {
    for attempt in 1 2 3; do
        if sudo timeout -k 10 120 apt-get "${opts[@]}" "$@"; then
            return 0
        fi
        echo "apt-get $1 failed or stalled (attempt $attempt of 3)" >&2
        # A kill during unpacking leaves dpkg wanting this before any
        # further install.
        sudo dpkg --configure -a
    done
    return 1
}

bounded update
bounded install -y --no-install-recommends "${packages[@]}"
