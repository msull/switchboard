## Earlier points

1. The excerpt: settled. It is kept in runtime core state, given a wire field, cut and tested, and the plan now says what it costs after a relaunch.
2. The literal count: settled. It now matches the worktree: ten in `src/app.rs`, one in the adapter.
3. The identity schema bump: settled. `CertificateReissued` now sets `schema_version`, and the core test starts from an identity at the old version.
4. The iPhone in step 0: settled. The iPhone run is in step 0, and the stop condition covers it.

## Cuts

- **Where this departs from the issue:** cut the "Scan the QR with a phone" bullet. Tapping the camera's link is how the issue's own scan works, so it is not a departure. The acceptance run (step 6.2) already says "tap the link".
- **The record, Rehydration:** cut it, or cut page steps 1–4 under **The UI**. The two give the same load order twice.
- **Decisions 5:** cut the sentences explaining what `include_str!` does. That explanation belongs to the reply to the user, not to the plan.
- **The actions and effects, `RemoteReply`:** cut the list of eleven line numbers. Adding the field makes every literal fail to compile until it has `set_cookie: None`, so the compiler lists them.
- **The actions and effects, `app.rs`, and Contract additions:** cut `info.capabilities` gaining `"device"`. The one reader of `/device` is the page, and the same host that serves the page answers it. The headset does not read it (step 3, Fixtures). A capability flag that nothing checks is a second mechanism.
- **The actions and effects, the excerpt, the Swift bullet:** cut Swift `SessionView.lastMessage`, its line in Contract additions, and the `lastMessage` check in step 5. The headset does not show the excerpt, and Swift's `Codable` decoding ignores unknown keys, so the rewritten fixtures decode without the field. Add it when the headset shows the excerpt.
