Changed: While the window is hidden, Switchboard checks tmux every 5 seconds instead of every second (hook events still update at once), and no longer runs `hostname` to name a new server identity.
