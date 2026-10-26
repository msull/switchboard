#!/bin/bash
# CI's apt step: the two packages the build links that the runner image
# lacks (cmake, which the whisper build needs, ships on it), with every
# apt call bounded by wall clock and retried. The Azure mirror dribbles
# or stalls: apt's own acquire timeout does not end a stall where the
# mirror holds the connection open and sends nothing (one `update` sat
# behind it for 86 minutes), and a 12 MB package once trickled through
# three two-minute tries. So the bound is `timeout` around the whole
# call, the packages are the two small ones, and the install reads the
# image's own package lists first: they are days old at most, and only a
# package that moved since the image was built (a 404) costs the 12 MB
# index fetch. A healthy run takes a few seconds.
set -euo pipefail

packages=(libxkbcommon-dev libasound2-dev)
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

install() {
    bounded install -y --no-install-recommends "${packages[@]}"
}

if ! install; then
    echo "installing from the image's lists failed; refreshing them" >&2
    bounded update
    install
fi
