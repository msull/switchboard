Fixed: The remote port no longer reads a request line or header without limit, so a stranger on the network cannot grow the app's memory; a paired device's WebSocket frame is capped at 16 KiB.
