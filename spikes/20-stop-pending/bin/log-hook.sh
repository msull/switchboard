#!/bin/sh
# Appends the event name and the full hook JSON to $SPIKE_LOG. Spike
# only: Switchboard's helper never logs a prompt.
event="$1"
payload=$(cat)
printf '%s\t%s\t%s\n' "$(date +%s)" "$event" "$payload" >> "$SPIKE_LOG"
exit 0
