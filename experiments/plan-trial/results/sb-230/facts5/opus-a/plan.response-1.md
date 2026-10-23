# Response to plan feedback 1

1. **The excerpt: you are right; the departure is gone and the plan
   builds the excerpt.** The spec already decides the source
   (`docs/remote-client.md:207`). The core gets the text in
   `EventKind::Stopped { last_message }` and drops it
   (`src/core/events.rs:132-135`). The agent excerpt comes from
   `SwitchboardApp::conversations`, not `UiState`.

   The plan settles where the value lives: in runtime core state,
   `AppCore.last_messages: HashMap<RecordId, String>`, not on
   `SessionRecord`. So there is no `SCHEMA_VERSION` bump and no
   `migrate` step. The cost is that after a relaunch a row has no
   excerpt until the session's next `Stop`. A persisted excerpt is
   named as a follow-up, with its bump written as "from the value at
   implementation time, plus one".

   Changed:
   - **Where this departs from the issue:** the excerpt bullet is
     removed.
   - **The record:** a new bullet gives the choice and its cost.
   - **The actions and effects:** a new "The excerpt" bullet.
     - The `Stopped` arm writes the excerpt into the map.
     - `session_view` fills the additive
       `SessionView.last_message: Option<String>`.
     - `changed {session}` comes from the existing diff, because
       `SessionView` derives `Hash`.
     - It takes the same `bound_strings` cut as `waiting_reason`.
     - The Swift `SessionView` gains `lastMessage`.
   - **The UI, page step 4:** a row shows the waiting reason when the
     session waits, otherwise `last_message`.
   - **The tests:**
     - step 1 adds a core test that extends `src/core/tests.rs:3354`;
     - step 3 adds a port test for the excerpt;
     - the "Fixtures are unchanged" line is replaced: `write_fixtures`
       seeds a `last_message` and the fixtures are rewritten;
     - step 5 checks that the Swift side decodes `lastMessage`.
   - **Docs:** the §4.3 sentence at `:207` is amended, and the design.md
     gap now reads "excerpt empty until the next `Stop` after a
     relaunch".
   - **Follow-ups for the owner:** the old excerpt follow-up is replaced
     by the persisted-excerpt one.
   - **Contract additions:** `SessionView.last_message` and its Swift
     counterpart.
   - The header line now says why `SCHEMA_VERSION` stays untouched.

2. **The literal count is fixed.** **The actions and effects**, in the
   `RemoteReply` bullet, now says 11 literals and lists them: ten in
   `src/app.rs` at 1664, 1679, 2365, 2371, 2376, 2754, 3570, 3723, 3736
   and 3767, and one at `src/adapters/remote.rs:829`. I checked these
   against the worktree. The figure of 28 had counted `-> RemoteReply {`
   signatures.

3. **The `IDENTITY_SCHEMA_VERSION` bump now means something.**
   - **The actions and effects**, `CertificateReissued`: the
     `update_identity` closure also sets
     `schema_version = IDENTITY_SCHEMA_VERSION`. The bullet says why:
     only `make_identity` writes the constant (`src/adapters/remote.rs:118`),
     and `update_identity` saves the clone at its loaded version.
   - **The tests**, step 1: the core test starts from an identity at the
     old version and asserts that the one `SaveIdentity` carries the new
     constant.
   - **Contract additions:** the `CertificateReissued` line says it
     sets `schema_version`.

4. **Step 0 now includes the owner's iPhone.** **The tests**, step 0
   adds a bullet: the owner's iPhone, against the same stub, runs For the
   owner items 1 and 2. It checks four things:
   - the click-through is offered;
   - the cookie is sent on the next `fetch`;
   - the cookie is sent on the `wss` upgrade;
   - the cookie survives closing Safari.

   The bullet cites the 2018 forum report as a lead only. The commit is
   not built past the stub until that run is in. The stop-and-report
   line now covers a failure on desktop Safari, Chrome or the iPhone.
   Decision 2 is unchanged: the cookie stays the default, and the
   subprotocol fallback stays a follow-up.
