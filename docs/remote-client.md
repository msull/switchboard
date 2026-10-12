# Switchboard Remote Client Architecture — v1

**Status:** Proposed implementation specification, revised against the repository
**Targets:** Switchboard Rust host; native visionOS client; reusable iPadOS client core
**Primary objective:** Browse and interact with the sessions and working sets of multiple independently running Switchboard instances, with native spatial windows on visionOS.

## 0. Revision notes

The first draft was written without the codebase. This revision merges its
concepts with what exists:

- "Cards" are not a resource. A working-set item is a `PinTarget` (a session
  record id, or a project id and a path relative to the project root) with a
  grid rectangle; the target is the item's identity. The API speaks
  **sessions, projects, working sets with pins, and files reachable only
  through a pin**. There is no `card_id`.
- The control port (`control/`, `switchboard-control`) already carries most of
  this API: `SessionView`, `SetView` with `Pin`s, `ProjectView`, `waiting`,
  `session.screen`, `session.send`, `session.resume`, `session.trust`, and
  `find` / `op.status`. The remote port is a transport over that contract,
  not a second one.
- The control port's operation id and `operations.log` are the idempotency
  mechanism, extended so that a non-replayable command is logged **before**
  it runs (today only creations are logged, and only after).
- The host has no Tokio and no HTTP stack. The core is single-threaded and
  every request is answered on the frame thread; the remote port keeps that
  shape, with bounded queues.
- The host has no event bus, and its transcript and screen polling follows
  the local view. The port adds a watch set of its own, app-level content
  caches that version what they read, a per-process revision counter and
  "changed" events with refetch, computed by diffing the exported read
  models after every pass rather than by watching for a save. Clients
  subscribe before they snapshot.
- Security is bound by the repo's existing rules (CLAUDE.md "Hard rules",
  design.md "Trust boundary"): one redaction policy covers every text that
  leaves the port and runs before any cut, host key material lives in the
  Keychain, the certificate
  is verified out of band before pairing, and agents are never resumed
  without a human.
- A spike precedes Milestone A, and no milestone ships a LAN-bound
  unauthenticated port.

The second review round (the outside reviewer's F1–F6, and a trim) changed
this:

- Collections are answered whole with bounded fields; the membership
  snapshot walk is gone. Only the conversation pages.
- The watch set is capped globally with least-recently-read eviction; there
  is no `watch_refused` and no "not live" client state.
- Content work (parsing, file reads, masking, hashing) runs on the
  connection thread, never on the frame; the frame hands out the handle and
  the policy and records the version. The frame does core state only.
- Content versions for files and logs are inode, ctime and size, not mtime
  and length (F2). Pairing codes, tokens, attempt counting and key rotation
  are specified (F3). The client keeps content in memory only (F5). A client
  workspace may span hosts (F6).
- Milestone A is five commits, A0 to A4, with a two-host read-only client at
  A2 (F1).

## 1. Product goals

- Keep agent execution, transcript parsing, filesystem access, and session ownership on the host machines.
- Support **multiple concurrently connected Switchboard instances** on a single client, not just a selected active host.
- Discover local hosts automatically; allow manually configured hosts.
- Provide unified working-set and session browsing, with explicit source-machine identity.
- Render native text and controls, not streamed desktop pixels.
- Preserve the existing presentation concepts: the compact working-set card, the full last-response view, and the complete navigable conversation for Claude Code sessions.
- Allow cards to open as independently movable native visionOS windows.
- Reuse the transport, models, authentication, and most SwiftUI components on iPadOS.
- Keep v1 local-network-first, but do not bake LAN-only assumptions into the API.

## 2. Non-goals for v1

- Central server, cloud account, cloud relay, or shared database.
- Cross-host session migration or execution of commands on a different host from the session's owner.
- Programmatic positioning of visionOS windows in 3D space.
- Automatic recreation of exact spatial coordinates.
- A new transcript parser. The host parses a transcript whole, as the app does today; §4.3 says what that bounds and what it does not.
- Editing working sets, projects, environments or definitions from the client.
- Answering Dispatch decisions from the client (the obvious v2; see §4.6).
- Full supervisor-driven workspace orchestration.

## 3. Core architecture

```text
Mac A: Switchboard + remote port --\
                                     \       Bonjour + HTTPS/WSS
Mac B: Switchboard + remote port ----> Native Client (visionOS / iPadOS)
                                     /       ConnectionManager
Mac C: Switchboard + remote port --/        UnifiedStore
                                             Window/PresentationStore
```

Each host is authoritative for its own resources. The client aggregates views without copying ownership of sessions. One client maintains **one connection state machine and one WebSocket per connected host**. A host going offline must not block other hosts.

### 3.1 Identity and namespacing

- `server_id`: stable, randomly generated UUID persisted in `<data dir>/identity.json` (its own small file beside `views.json`, so it needs no `SCHEMA_VERSION` bump; written through the store's atomic path). Never derive it from hostname, IP, or Bonjour instance name. A second instance pointed at another `SWITCHBOARD_DATA_DIR` therefore has its own.
- `identity.json` also holds the host's **certificate** (DER, base64) and the certificate's creation time. The certificate is public; its private key is in the Keychain (§5). The two are made together, once, on first listen. If the key is missing when the certificate is present (a Keychain reset, an unsigned build), the host makes a fresh pair, logs that it did, and the paired-devices dialog says every pairing is now invalid.
- `server_name`: user-editable display name, kept in `Settings` (one new defaulted field; this one does take a `SCHEMA_VERSION` bump and a migrate step, per the repo rule).
- Host-side identifiers are the existing ones and are never renamed on the wire: `RecordId` (a session, stable forever), `ProjectId`, `SetId` (a working set, in `views.json`), `SpaceId`. The host id (the tmux session) and the resume handle (the provider's session id) are never exposed as identity; `resume_id` is informational, as on `SessionView`.
- All client-facing references are **qualified**: `(server_id, resource_id)`. A `SessionRef` is `{server_id, record_id}`, a `WorkingSetRef` is `{server_id, set_id}`, a `FileRef` is `{server_id, project_id, path}`.
- A single client may show items from multiple servers in one collection, but every command is routed to the item's owning server.
- `server_id` identifies a logical installation, **not a trusted cryptographic identity**. Trust is the pinned public key established at pairing (§5).

### 3.2 Multi-instance behaviors

- Maintain a list of **known servers** (paired and manually added), independent of currently discovered servers.
- Discovery adds candidates; it does not automatically trust or authorize them.
- Client can connect to all enabled known servers concurrently, reconnecting each independently.
- Default unified view includes items from all connected servers; filter by one or more servers.
- Show a source-machine label on each card, with a clear offline/stale state.
- When a host disconnects, keep its last known items visible as **read-only cached snapshots** with a disconnected indicator. Do not imply that actions succeeded.
- When a host reconnects, reconcile the host's authoritative state before re-enabling commands.
- Preserve the selected server filters and window/item references across client relaunches.
- If two endpoints advertise the same `server_id`, treat them as candidate addresses for one logical server; verify the pinned key before switching endpoints. Do not silently merge untrusted installations.

## 4. Rust host

### 4.1 Implementation

- **No Tokio.** The host is std threads and `mpsc`; the core is owned by `SwitchboardApp` and every state change is a dispatch on the frame thread. The remote port follows the `ControlSocket` shape: an accept thread per listener, one thread per connection, each request handed over a channel to the frame, answered by the same `serve` path in `app.rs` (`serve_pending` runs before every frame, hidden ones included), the reply sent back on the connection's channel. A second runtime inside an egui app buys nothing for a handful of connections.
- **The frame does core state only.** A metadata request (a view, a command) is answered on the frame as the control port is. A content request (conversation, screen, run output, file) takes two steps: the frame answers with the **handle and the redaction policy** (the transcript path and resume handle, the pane's host id, the run's log path, the file's root and relative path, plus the secret values to mask and the policy fingerprint), and the **connection thread** does the read, the parse, the mask, the version check and the serialization, then sends one small action back to the frame to record the version it observed (§4.4). The watch poll (§4.4) runs on one worker thread of its own the same way. A multi-megabyte transcript, a 768 KiB file or a slow Keychain call therefore costs a connection thread its time, never a frame. The handle is read under the frame's authority, so a record removed or rebound between the two steps makes the worker's version action a no-op and the reply carries the handle it read from.
- **Bounds.** The port is one user's few devices, and the limits say so:
  - at most 16 connections in total and 4 unauthenticated ones; a further accept is closed at once. The 4 are a separate pool: an unauthenticated flood fills its own slots and never a paired client's;
  - `/pair` is answered at most six times a minute per listener and `/info` sixty; past that the connection thread answers 429 without reaching the frame;
  - a TLS handshake and the first request line each within 5 s, a request body within 10 s, a write within 10 s; a connection that misses a deadline is closed;
  - one request in flight per connection (HTTP/1.1, no pipelining);
  - the frame drains at most 8 remote requests per pass, in arrival order, the rest waiting on the channel; the channel is bounded at 64 and a full channel answers 503 from the connection thread without reaching the frame; a frame-side request is a view or a handle lookup and takes microseconds, so the count is the bound;
  - each WebSocket subscriber has an event queue of 256; when it fills, the subscriber is sent `resync_required` and closed, never silently trimmed (§4.4);
  - the watch set (§4.4) is capped at 128 keys for the whole port; past the cap the least recently read key is evicted silently, since a read of an evicted key always works and re-enters it.
- Preferred crates: a blocking HTTP/1.1 server over `std::net::TcpListener` (hand-rolled or `tiny_http`), `tungstenite` for WebSocket, `rustls` with `rcgen` for the per-installation certificate, and `mdns-sd` (or the system `dns-sd` through a small macOS adapter) for Bonjour. All of it in `src/adapters/remote.rs` behind a `ports::remote` trait with a fake, and all of it must compile on Linux CI (Bonjour may be a `cfg(target_os = "macos")` adapter with a no-op elsewhere). **Spike 15 (§10) decides the exact crates before any of this is built.**
- Handlers are thin: HTTP request → auth and scope check → allowlist → `wire::Request` → `serve` → redaction → `wire::Reply` → HTTP response. No session or transcript logic lives in the adapter.
- Only the instance holding the store lock listens (as the control port does); a read-only instance refuses to bind.
- Bind to loopback by default until remote access is explicitly enabled in Settings. For LAN access, bind the chosen interface and require pairing.
- Configurable port in `Settings`, default `43187`, overridable with `SWITCHBOARD_REMOTE_PORT` (a dev aid, documented in the README) so two instances can run on one machine.
- Advertise `_switchboard._tcp` via Bonjour only while listening on a non-loopback interface. TXT records carry the protocol major version and the display name; never a token, never key material.
- If Switchboard closes, the port goes away. A standalone daemon is not in scope.

### 4.2 API conventions

- Version prefix: `/api/v1`.
- JSON request and response bodies. Timestamps as milliseconds since the epoch, matching the wire crate (`last_stop_at_ms`); the client formats them.
- The wire crate's views are the response schemas. New views are added to `switchboard-control` additively (it stays std + serde only, and Dispatch keeps compiling).
- Standard error shape: `{ "error": { "code": "...", "message": "..." } }`, with `wire::Reply::Failed` mapped onto it.
- **Every response** carries the host's `epoch` and `revision` (§4.4) in an envelope: `{ "epoch": "...", "revision": n, "body": ... }`. The revision is the one at which the body's **content version** was observed (§4.4, "Content versions"), not merely the counter at the time of the reply. The client keeps one held revision per resource key (a session's metadata, its conversation, its screen, one run's output, one file are separate keys) and applies a reply only if its revision is at or past the held one, so an older reply arriving after a newer one is dropped and fetching a session's metadata never invalidates a valid conversation page.
- **Caps.** The serialized, masked body of any reply is at most 1 MiB, envelope included, enforced on the output bytes. The rule per kind:
  - **collections** (`projects`, `spaces`, `sets`, `sessions`, `runs`, a set's pins, `waiting`) are answered **whole, in one reply, never paged and never cut**. They are small: a data directory holds hundreds of records at most, and a project's session list with every string bounded as below is under 100 KiB at that scale. A collection reply is one snapshot of its membership at one revision, which is what makes a cached member's removal safe: a member missing from a whole reply at a revision at or past the one it was learned at is gone. The 1 MiB cap is enforced on the serialized reply as everywhere; a collection that would exceed it is a bug in the bounds, answered `too_large` and logged, not paged around.
  - **metadata views**, in a collection and standing alone, are bounded field by field, after masking: `notes` at 4 KiB in a collection and 64 KiB standing alone, `name`, `waiting_reason`, `cwd` and every other free-text string at 4 KiB, each cut marked in a `truncated` list of field names; no field is left unbounded. A client that wants a session's full notes reads the session alone.
  - **conversation** pages stop at `limit` turns or at the cap with `next`; a turn larger than 256 KiB is cut and marked (§4.3).
  - **last response** and **run output** are cut from the front (older text dropped) and marked `truncated: true`.
  - **file content** is sent whole or not at all: the port's file limit is 768 KiB of source bytes, and a larger pinned file answers `too_large` with its size. The source limit is not a guarantee of fit: JSON escaping can double a file of quotation marks and masking can lengthen text, so the limit is enforced once more on the masked, serialized reply, and a file that exceeds it there answers `too_large` too, naming the serialized size. The client says so on the card.
  - **screen** is bounded by `lines` (at most 200) and, after masking, by bytes: text over 256 KiB is cut from the top (older lines dropped) and marked `truncated: true`.
- A command is **accepted** when `serve` returns; its outcome arrives through events and the session's state. For `session.send` there is no completion signal beyond the next `Stop` hook event on that record (§4.5).
- Document payload schemas with examples; generate the Swift `Codable` types from the Rust views (serde shapes) and keep a fixture test on both sides.

### 4.3 Endpoints

| Method | Endpoint | Wire body | Scope |
|---|---|---|---|
| GET | `/api/v1/info` | new: `info` | none (identity only, before pairing) |
| POST | `/api/v1/pair` | pairing handshake (§5) | none, only inside the pairing window |
| GET | `/api/v1/projects` | `projects` | read |
| GET | `/api/v1/spaces` | `spaces` | read |
| GET | `/api/v1/spaces/{id}/sets` | `sets` | read |
| GET | `/api/v1/sets/{id}` | new: `set` (the `SetView` with pins) | read |
| GET | `/api/v1/projects/{id}/sessions` | `sessions` | read |
| GET | `/api/v1/sessions/{id}` | `session` | read |
| GET | `/api/v1/waiting` | `waiting` | read |
| GET | `/api/v1/sessions/{id}/screen?lines=n` | `session.screen` | read |
| GET | `/api/v1/sessions/{id}/conversation?from=turn&limit=n` | new: `session.conversation` | read |
| GET | `/api/v1/sessions/{id}/last-response` | new: `session.last_response` | read |
| GET | `/api/v1/sessions/{id}/runs` | new: `session.runs` | read |
| GET | `/api/v1/sessions/{id}/runs/{n}/output?lines=n` | new: `session.run_output` | read |
| GET | `/api/v1/projects/{id}/files/{path}` | new: `file.read` (pinned files only) | read |
| POST | `/api/v1/sessions/{id}/send` | `session.send` | control |
| POST | `/api/v1/sessions/{id}/resume` | `session.resume` | control |
| POST | `/api/v1/sessions/{id}/trust` | `session.trust` | control |
| GET | `/api/v1/ops/{op}` | `op.status` | read |
| GET | `/api/v1/events` | WebSocket upgrade | read |

Everything else on the wire is **refused before translation**, by an allowlist, not a denylist: `env.resolve` returns secret values; the `env.*` writes, `definition.install`, `project.add`, `project.root`, `set.sync`, `workflow.*`, `dispatch.runner`, `session.kill` and `session.remove` change what runs on the host or what it keeps. v1 remote is the table above and nothing more.

**Redaction, one policy for every text that leaves the port, applied before any cut.** `session.screen` today masks the values of the project's secret variables and of the session's granted environment sets, and it reads slack lines above the window first so a value wrapped across the window's first line is still found whole. The port generalizes both halves of that: one `redact` function in `app.rs`, and the rule that **masking runs over complete source text, and every cut comes after**, since a value split by a cut no longer matches. Per kind:
  - **conversation**: the parser caps tool detail at `DETAIL_CAP` while parsing, so masking a parsed turn would miss a value crossing that cap. The transcript reader gains an additive `read_masked(handle, &dyn Fn(&str) -> String)` that applies the mask to every string field before `cap`; the port's conversation cache (§4.4) is populated through it, keyed by the secret set's fingerprint as well, and is separate from the UI's unmasked cache. The 256 KiB turn cut then runs on masked text.
  - **run output**: `scrollback::tail_text` cuts bytes before stripping escapes and choosing lines. It gains an overlap: the read starts `longest secret length` bytes before the tail window, the mask runs over the whole buffer, and the line selection then discards the overlap; a value straddling the window's first byte is therefore masked whole. A secret value containing a newline is masked only within a line, which the dialog's sentence on redaction covers.
  - **screen**: the existing slack-lines read, unchanged.
  - **file content**: the file is read whole under its limit, masked, then sent.
  - **last response** and the card excerpt: masked from the turn's full text, then cut.
  The secret set for a session is the project's secret variables plus its granted sets; for a file it is the owning project's. The set is resolved **before any cached content is returned**, on every read, and its **policy fingerprint** (a hash over the sorted names and values, held in memory only, never logged or stored) is part of every masked cache's version (§4.4): a new grant, a changed value or a removed set makes previously masked bytes stale even when their source did not move. If the secret set cannot be resolved (a Keychain error, a missing set), the reply is **refused** with that reason, never sent unredacted and never served from a cached success. What this covers is exactly the values Switchboard holds: a secret the user pasted into a prompt, or one a tool printed that Switchboard never knew, is not found. The paired-devices dialog says this in a sentence, and the claim in design.md's trust boundary is widened to "or as redacted text on the remote port".

New read models, added to the wire crate:

- `session.conversation`: the `Conversation` from `ports::transcript` as a wire shape: title, model, branch, usage, and turns paged by **turn number** (`n`, 1-based). The host parses the whole JSONL, as the session view does, from the app-level cache (§4.4); a transcript the app can show is one the port can serve, and the supported size is whatever the app handles, which the spike measures. What paging bounds is the **response**: a page stops at `limit` turns or at the body cap, whichever first, and says `next` for the turn to continue from. A single turn larger than 256 KiB is cut (final text kept whole where possible, activity text trimmed) and marked `truncated`. The page carries the `resume_id` it was read from: `/clear` and a discard rebind the record to a new transcript, and a client holding turn numbers from another handle drops them and refetches from 1. The last turn is always in progress until a `Stop` event: on a `changed {conversation}` the client refetches from the last turn it holds and **replaces** that turn and everything after it. A handle rebind (`resume_id` moved on the session view) changes the conversation cache's key, which emits `changed {conversation}` of its own, so the client needs no rule linking the two. Only Claude Code sessions have a conversation; Codex, shell, command and service sessions answer with a "no conversation" reason and the client falls back to `screen` (and, for commands and services, `run_output`).
- `session.last_response`: the last turn's `final_text`, its `n` and `end`. It is a **projection of the `conversation` key**: it is read from the same cache, carries the conversation's content revision, is watched by watching `conversation`, and refreshes on `changed {conversation}`; a window showing only the last response registers that watch. The compact card's one-line excerpt is not part of it: that is `last_message` from the `Stop` hook on the record, a field of the session's metadata, and moves with `changed {session}`.
- `session.runs`: a command's or service's runs as the record holds them (`Run`: `n`, started, ended, exit, artifacts' names, not their paths). `session.run_output`: the tail of one run's scrollback log (`scrollback::tail_text`, escapes stripped, `lines` clamped to 500), redacted; a run whose log was pruned or rotated answers with that reason. This is how an exited command whose pane is gone is still readable: `session.screen` refuses a session that is not running, by design.
- `file.read`: content of a file **only if some working set pins it** (`PinTarget::File` with that project and path). The read goes through a new `ports::files` method, `read_pinned(root, relative)`, not through the UI's preview loader. Confinement is by descriptors, not by path strings, so it stays valid while names change under it:
  - the relative path is rejected if it is absolute, empty, or has a `..` or `.` component, before anything touches the disk (a persisted pin can hold anything);
  - the project root is opened once as a directory descriptor (`O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC`) and the walk proceeds **one component at a time** with `openat` from the previous descriptor: every directory component with `O_DIRECTORY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, the last with `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`. A component that is a symlink fails the open (`ELOOP`), so **no symlink anywhere in the path is followed, parent or leaf**; a swap of any directory for a link between two hops is refused at the next hop, and a swap after the hop cannot move the descriptor already held. `openat` comes through a safe wrapper crate (`rustix`; or `cap-std`, which does this walk itself, if the spike prefers it); the crate-wide `unsafe` ban stands;
  - `O_NONBLOCK` on the open is what keeps a FIFO from parking the connection thread for good: the open returns at once, `fstat` on the descriptor must say regular file, and anything else (a FIFO, a socket, a device, a directory as the leaf) is refused before any read. The flag is cleared on the regular file before reading. The read runs on the connection thread (§4.1), so even a stall here costs one connection, never a frame;
  - the read is `take(limit + 1)` with the port's limit of 768 KiB (§4.2), so growth after the check cannot exceed the limit;
  - dotenv names (`.env`, `.env.*`) are refused whatever is pinned, since those hold the project's secrets;
  - the reply carries the file's `modified` and `size` from the same descriptor, and the content is masked like every other text.
  Both platforms the crate builds on are Unix, so one implementation serves macOS and Linux CI.
- `info`: `server_id`, `server_name`, protocol version, capabilities, `epoch`, `revision`, the certificate's public-key hash (SPKI, SHA-256), the port's limits (file limit, conversation page size, body cap), and whether the pairing window is open.

Avoid a general-purpose filesystem endpoint. Paths are never accepted from the client except as the identity of an existing pin.

### 4.4 WebSocket events

Envelope:

```json
{
  "server_id": "uuid",
  "epoch": "uuid",
  "revision": 1042,
  "type": "changed",
  "resource": {"kind": "session", "id": "record-uuid"},
  "at_ms": 1760220000000
}
```

Event types:

- `hello {epoch, revision}`: the first frame on every subscription.
- `changed {kind, id}` with the **metadata kinds** `project`, `space`, `set`, `session`; the **content kinds** `conversation`, `screen`, `run_output` (`id` = `record/n`), `file` (a pinned file, `id` = `project_id/path`); and the collection kinds `projects`, `spaces`, `sets` (with the space's id), `sessions` (with the project's id), `runs` (with the record's id), `waiting`, emitted when a collection's member id set moves and answered by refetching the collection whole. Each kind is its own resource key on the client (§4.2).
- `removed {kind, id}`: the resource is gone; the client drops it and marks any window on it "removed on <server>".
- `watching {kind, id}`: the answer to a `watch` frame (below).
- `resync_required`: sent before the host closes a subscriber whose queue overflowed; the client reconnects and refetches everything.

**What produces events.** The host has no event bus, and a save is not a reliable sign of a visible change: working sets save through `SaveViews`, Claude Code's trust question (`PromptSeen`) and a Codex pane going quiet (`HostListed`) change what a card says with no save at all. So the port does not watch for saves. Revision and events come from three places in `app.rs`:

1. **The read-model diff, after every pass.** After every `dispatch` and after every polling pass (`HostListed`, the hook log, `Tick`), the app recomputes a cheap per-resource fingerprint of each exported metadata read model (`SessionView`, `SetView` with its members, `ProjectView`, `SpaceView`, `waiting`) and compares it with the previous pass's. One `changed` per resource whose fingerprint moved, one `removed` per resource that vanished, one `changed` per collection whose membership moved, and `revision` bumped once per pass that emitted anything. The diff is the gate, not the effect list, so a set rename, a pin edit, a trust-question transition, a quiet-state transition and a time-based rule-set change all surface without anyone naming them. The fingerprint is a hash over the view's serialized bytes **with its time-derived fields held at zero**: `quiet_secs` is computed from the clock in `session_view` and would move every pass, so the view gains an additive `last_activity_ms` the client derives quiet from, and the quiet **state** (the card's word moving to idle) is what the diff sees. Collections carry the membership revision of §4.2, bumped only when the member id set moves. The hash is cheap at this scale (hundreds of records); the spike measures it (§10).
2. **Content versions, assigned where content is read.** Conversation text, pane text, run output and pinned files are mutable outside the core and can change between two reads inside one poll interval, so a content reply's revision cannot come from the counter alone. Every content read goes through a per-key **content cache** in `SwitchboardApp`, whose bookkeeping lives on the frame and whose reads run on the connection or worker thread (§4.1). The cache holds, per key, the **source version** and the **policy fingerprint of the owning project (§4.3)** it last served, with the revision it was assigned. The source version per kind, chosen so that no change the host can see leaves it unchanged:
   - **transcript**: size and mtime. Claude Code's JSONL only appends, Switchboard never edits one, and `/clear` or a discard rebinds the record to another file, so a growing size is a reliable version and a same-size rewrite does not occur;
   - **pinned file**: inode, ctime and size, read from the descriptor the walk ends on. No user tool preserves ctime, so an editor that rewrites in place, a same-size rewrite, and a replace-by-rename (a new inode) all move it;
   - **run output**: inode, ctime and size of the run's log. A log truncated and regrown to the same length has a new ctime; a rotated one a new inode;
   - **screen**: a hash of the pane text, as today.
   When either the source version or the policy fingerprint differs from what the cache holds, the worker's reply carries the new content and the frame, on the worker's version action, bumps `revision`, emits `changed {content kind, id}`, and stores the new version with that revision; the reply is stamped with that revision. **Two different content versions never share a revision for one key within an epoch**, and two reads of unchanged content return the same bytes and the same revision. The policy is resolved before the read; a resolution failure refuses the reply and leaves the cache untouched, so no earlier masked bytes stand in for the answer. A policy change is also noticed by the diff pass (the `env.*` actions are dispatches), which emits `changed` for every watched content key of the affected project without waiting for a read.
3. **The watch poll.** The port keeps a **watch set**: every content key any subscriber has asked to watch, plus every content key read in the last minute. Independently of the local view, every two seconds a worker thread checks each watched key's source version (a `stat`, or a pane snapshot for `screen`; never a parse) and, for a key that moved, reads it through the content cache, which is what emits the `changed`. The local session view reads its conversation through the same cache (an unmasked variant; §4.3), so the two polls share work rather than duplicate it. The set is capped at 128 keys (§4.1); past that the least recently read key is dropped, and a key with no subscriber and no read for a minute is dropped too. A read of a key not in the set still answers and still versions correctly, since the version is checked on every read; what eviction costs is the `changed` event until the next read, which the client's periodic refetch of a visible view covers.

**Watch frames.** A client sends `watch {kind, id}` when the first view instance on a content key opens and `unwatch` when the last one closes; the client keeps a reference count per key so closing one of two windows on a session changes nothing on the host. The host answers `watching` once the key is in its set, and the client fetches the initial content only after that, so the first snapshot's revision is covered by the poll from then on. A watch is never refused. On reconnect, after `hello`, the client resends every watch with a nonzero count before it snapshots. A visible content view also refetches on its own every thirty seconds, which is what covers a key the host evicted from the set.

**The conversation cache** moves from `UiState` to `SwitchboardApp` as one of the content caches, keyed by record id, resume handle and content version, read by the session view and the port alike; `UiState` keeps a reference, not a copy. This is a frontend change (the session, card and workflow views read the cache), so it is its own commit before any port code, Milestone A0 (§10), with the app behaving identically. This, the diff and the worker handoff are the changes to the app outside the adapter, and §12.6 says why they are required.

**Subscribe first, then snapshot.** A client connecting to a host:

1. opens the WebSocket and waits for `hello {epoch, revision}` (events start flowing at once and are buffered by the client);
2. fetches the snapshots it needs, each reply carrying its own `revision`;
3. applies the buffered events: an event whose `revision` is past a snapshot's `revision` invalidates that resource and is refetched; the rest were already in the snapshot;
4. enables commands.

Because the subscriber is registered on the frame thread before any later pass can bump `revision`, a metadata snapshot is taken on that thread with the revision it carries, and a content reply is stamped on that thread when the worker's version action lands (the same moment its `changed` would be emitted, so the event and the reply name one revision), no change can fall between a subscription and a snapshot. Several snapshots in flight are each reconciled against the buffer by their own revision; events that arrive during a refetch are reconciled the same way, which is why the client never clears its buffer until step 4. A resource named by an event the client has never seen is fetched. If `epoch` differs from the one the client holds (the host restarted), every cached item is stale and is refetched. There is no retained-event replay: a `resync_required` is the only answer to a gap, and events are never durable.

### 4.5 Commands

- `send`, `resume` and `trust` are authenticated actions under `control` scope against one owning server and session.
- The client makes a UUID operation id per command and sends it as the wire `op`.
- **Logged before run.** Today `operations.log` gets a request line only for creations, and the reply line after the command ran; `serve` skips a command only when a reply line exists. That leaves a crash between sending keys and appending the reply as a repeat on retry. The port closes that gap for every non-replayable command (`Class::NonReplayable`: `session.send`, `session.resume`): `serve` appends the request line **before** dispatch, and a failed append refuses the command without running it. On arrival, an op with a request line and no reply line is answered `interrupted` and **not run again**; an op with a reply line is answered from it. `session.trust` is `Idempotent` on the wire and needs neither.
- The client's rule per `/ops/{op}` status:
  - `done`: use the reply;
  - `interrupted`: the host started it and the outcome is unknown; never resubmit; show the session's current state and let the user decide;
  - `unknown`: the host never received it (nothing was logged, and the frame answers queued requests in arrival order, so a request that had reached the frame before the status query has a line); resubmitting with the same op is safe.
  A timeout on a command is followed by a status query, never by a blind resend.
- `send` puts keys into the pane, as the app's own message box does. "Accepted" means the keys were sent. Progress is the record's state: `PromptSubmitted`, then working, then `Stop` with the new last response. A send to a pane sitting at Claude Code's folder trust question is accepted but goes nowhere until `trust` answers it; the session view carries `trust_question`, and the client shows the same "Trust this folder" button the app shows.
- `resume` on a not-running agent is user-initiated, so allowed, but the client confirms it ("Resume spends a turn") before sending; the host never resumes on its own.
- Failures name the reason: not running, not resumable, unknown record, scope missing, pairing revoked, log unavailable.

### 4.6 Dispatch and waiting

"Waiting on you" on a card includes reasons set from outside: Dispatch marks a session through `session.waiting` when one of its tickets needs a decision. `SessionView.waiting_reason` is already on the wire; the client shows it. Answering a Dispatch decision remotely is v2, through Dispatch's own port, not through Switchboard. The `Dispatch · <project>` working sets are ordinary sets to the client.

## 5. Security and pairing

A local network is not a trust boundary: these APIs can instruct coding agents and expose private transcripts. The repo's rules apply unchanged: secret values never reach a record, a log line, a command line, or any reply but `env.resolve` (which is not on this port) and, as redacted text, the read models of §4.3; the Keychain holds key material.

- Require pairing **before** anything but `/info` and `/pair`.
- **Host identity**: a self-signed certificate made once with `rcgen` on first listen. The certificate is in `identity.json` (§3.1); its private key is a Keychain item through the existing adapter (`adapters/keychain.rs`, service `com.sadburger.switchboard`) under the account `remote-key:<server_id>`, so two data directories under one macOS account never overwrite each other's keys. The bundle must be signed with the stable identity (`scripts/bundle.sh`) or the Keychain ACL is lost on every rebuild.
- **What the client pins** is the SPKI SHA-256 of the host's public key, not the certificate's bytes, so a certificate reissued from the same key (a new expiry) still verifies. Rotating the key is making a new identity: every pairing is invalid and the dialog says so.
- **Pairing verifies the key before the code is sent.** The code alone would let an intermediary terminate the first connection, forward the code to the real host, and take the token. So the pairing window shows a **QR code** carrying `{server_id, spki_sha256, code, addresses}`, and under it the same in text: a short form of the key hash and the code. The client scans or types both; it connects, compares the hash of the key it is actually talking to with the one it was given, and only when they match sends `/pair {code, device_name, spki_sha256}`. The host refuses a pairing whose `spki_sha256` is not its own, so a relayed request through a substituted certificate fails even if the code reached it. Before pairing, `/info` is served to any client and nothing in it is trusted; after pairing, every connection verifies the pin before the token is presented.
- The pairing window is opened from the settings menu as "Allow pairing for 5 minutes", the same shape as the environment setup window (`ENV_SETUP_WINDOW`, never saved). The dialog that opens it chooses the scope the pairing grants, `read` by default.
- **The code**: eight characters from the 32-character Crockford alphabet (no `I`, `L`, `O`, `U`), 40 bits, drawn from the OS random source (`getrandom`), made when the window opens and shown as the QR's `code` and in text. It is single-use: the first successful `/pair` consumes it and closes the window. **Failures count globally**: one counter in the app's state, bumped on the frame for every failed `/pair` from any connection, and the window closes at three. Concurrent attempts cannot each get three, because the frame answers them in arrival order against the one counter.
- **The token**: 32 bytes from the OS random source, base64url, 43 characters. It does not expire; the owner revokes. It is sent only as a bearer header over the pinned connection, never in a URL.
- `/pair` answers with the token and the granted scope. The host stores the device name, scope, created time, last-seen time and the token's SHA-256 in `<data dir>/devices.json` (atomic write, owner-readable); never the token. The client stores the token and the pinned key hash in its Keychain.
- **Scope changes** happen in the host's dialog: a device's scope can be raised to `control` or lowered to `read` there, taking effect on the device's next request. There is no scope request over the port.
- **Key rotation** (a missing key at listen, §3.1) **deletes every record in `devices.json`** when the new identity is made, so an old token stops working on the first request, and the dialog says the pairings are gone. A client seeing a new key for a known `server_id` shows "identity changed, pair again" and sends nothing, token included.
- Every later request carries the bearer token; the host answers with the device's scope. A Settings dialog (built the way the Environment dialog is) lists paired devices with scope, last seen, and Revoke, and states what redaction does and does not cover. Revocation removes the record, takes effect on the next request and closes the device's WebSocket.
- The client verifies the pin on every connection and on endpoint changes; a mismatch is shown as "identity changed, pair again" and never auto-merged.
- Validate input, cap request bodies at 64 KiB, cap `lines` on `screen` (the wire clamps to 200) and `run_output` (500), cap pages and bodies as §4.2 says, and keep tokens out of logs, URLs, Bonjour TXT records and crash reports. Refusals are logged by op and kind, never with content.
- Loopback-only without auth is acceptable **only inside the spike directory**. No milestone commit binds a non-loopback interface without pairing.

## 6. Discovery and connection lifecycle

### 6.1 Discovery

- The host advertises `_switchboard._tcp` while listening on a LAN interface.
- The Apple client browses with `Network.framework`.
- `NSLocalNetworkUsageDescription` and `NSBonjourServices` (`_switchboard._tcp`) in the Apple app manifests.
- Manual host and port entry for networks where mDNS is blocked.
- Known servers and pinned identities are persisted separately from transient discovery results.
- A changed IP address is not a new server.

### 6.2 Per-host state machine

`discovered -> unpaired -> pairing -> connecting -> syncing -> connected -> reconnecting/offline`

- A failed connection to host A never blocks host B.
- Bounded exponential backoff with jitter; reconnect promptly on network changes and on the app returning to the foreground (visionOS and iPadOS drop the socket in the background; cached snapshots cover the gap).
- On reconnect: verify the pin, present the token, check `/info` for protocol compatibility and epoch, subscribe, snapshot, reconcile (§4.4), then enable commands.
- Errors surface per host.
- Unsupported protocol versions are shown explicitly.

## 7. Swift client structure

Suggested modules:

- **SwitchboardProtocol:** Codable models mirroring the wire views, typed requests, version negotiation, the event envelope.
- **SwitchboardTransport:** HTTPS/WSS transport with the pinning delegate (`URLSession` and `URLSessionWebSocketTask` share it), bearer auth, reconnection, discovery.
- **SwitchboardClientCore:** `ServerRegistry`, `ConnectionManager`, per-server stores with the event buffer and revision reconciliation, `UnifiedStore`, command routing, cached snapshots with a per-server budget (pages of a conversation evicted furthest from the view first).

**What the client keeps, and where.** Persisted, in the app's container: the known-server list (`server_id`, display name, last addresses, enabled flag) and the user's presentation state (workspaces, view instances, window ids, per-view mode and position, server filters). In the Keychain: the bearer token and the pinned key hash per server. **In memory only**: every read model and every content body (session metadata, conversations, last responses, screen text, run output, file content) and any unsent draft. Nothing from a host is written to disk, so there is no data-protection class to choose and no purge schedule; relaunching the client starts from the server list and refetches. Offline, the cached snapshots of §3.2 are what memory holds since the last connection, and they go with the process. Per-server memory budget: 32 MiB of content, evicted furthest from any open view first. **Forget server** removes its token, pin, server entry, memory and every view instance on it; **revocation** or an **identity change** seen on a connection drops the token and the memory at once, marks the server as needing pairing, and keeps the view instances so they can be refilled after a new pairing. A cached body is never shown as current without its connection: every disconnected view says so.
- **SwitchboardUI:** shared SwiftUI card, last-response, conversation, run-output, working-set, server selector and connection status views.
- **SwitchboardVision:** visionOS entry, spatial `WindowGroup`s, card pop-out lifecycle.
- **SwitchboardPad:** iPadOS entry and platform-appropriate multiwindow navigation.

One `ServerConnection` actor per known host. The aggregate store exposes qualified references and derived views. UI never builds host URLs to fetch an item.

### 7.1 Presentation model

Separate:

1. **Resource:** a session, working set or pinned file owned by a server.
2. **View instance:** a presentation of that resource, with mode (`compact`, `last_response`, `conversation`, `screen`, `run_output`), selection, scroll and disclosure state, and a unique view-instance id. The mode set depends on the session's kind: `conversation` and `last_response` exist for Claude Code sessions, `screen` for every running session, `run_output` for commands and services.
3. **Workspace:** a saved collection of view instances and client-side organization. **A workspace is cross-host by construction**: its view instances refer to qualified resources, so one workspace may hold views on any combination of paired servers, with no coordinating server and no host-side resource for it. A host's working sets stay host-owned (§8); a workspace is the client's own grouping over them.

The same resource can be shown in several views without duplicating anything. Client-side window and view state is never sent to the host. The per-server store counts the view instances interested in each content key and sends `watch` and `unwatch` only on the transitions between zero and one (§4.4).

### 7.2 visionOS windows

- Main window: combined browsing with list, grid and focus modes and server filters. Card sizes follow the host's grid rectangles only as a hint; the client lays out for its own space.
- Card window group: `WindowGroup(for: CardWindowRequest.self)` keyed by a stable window-instance id and a qualified ref.
- `openWindow` and `dismissWindow` create and close windows; the system positions them.
- Closing a window never stops a session.
- Per-view mode and navigation state survive, keyed by `(resume_id, turn n)` for conversation position.
- When a server goes offline, windows stay open on cached content with controls disabled. A `removed` event marks the window and disables its controls; the window stays until the user closes it.

### 7.3 iPadOS reuse

- Reuse protocol, transport, stores, presentation models and most views.
- Adapt multiwindow behavior to iPadOS scenes.
- No RealityKit or visionOS-only APIs in shared packages.

## 8. Unified browsing and filtering

- Default view: all connected hosts, with source labels.
- Filter by server, working set, session kind, and state (`CardState` labels: waiting on you, working, idle, starting, exited, not running, not resumable).
- Sorting across hosts: the host's own rank (waiting first, then working, starting, idle, exited, not running, mirroring `CardState::rank`), then last activity, then record id as the tie-breaker.
- Server management screen: discovered and known servers, status, pairing, rename, enable/disable, forget.
- Working sets with the same name on different servers stay separate by qualified id; the UI groups by source.
- No client-made cross-host working set in v1. The host's sets are read-only from the client.

## 9. Remote access later

Every server is an authenticated endpoint; nothing assumes RFC1918 addresses or Bonjour. A future Tailscale or WireGuard route, a manual remote endpoint, or an outbound relay reuses the same protocol, token and pinned key. No relay in v1. The pin is bound to the installation's key, not to an address or a certificate's validity window, so a route change or a reissued certificate does not invalidate it.

## 10. Delivery plan and acceptance criteria

### Spike 15 — `spikes/15-remote-port`

Before any host code, with measured results in its README:

- A blocking HTTPS server over `std::net::TcpListener` with `rustls` and an `rcgen` certificate, a WebSocket upgrade with `tungstenite`, and requests handed to a stand-in "frame" thread over a bounded `mpsc`, as the control socket does. Measure request latency through the handoff at the app's frame rate while hidden.
- **Frame cost, measured, with targets.** The frame's share of a metadata request, a content handle lookup, a worker's version action and the read-model diff over a few hundred records, each timed per pass; the target is under 1 ms per pass for the port's whole share, and under 4 ms at the 99th percentile with 8 requests in a pass. The content work on the connection thread timed separately: the largest transcript in the user's data directory parsed and masked (its time and peak memory are the supported size for §4.3), a 768 KiB file read and masked, a Keychain read made slow (a locked keychain) and shown to stall only its connection. A stalled subscriber and 20 connections that stall after the TLS handshake, with a healthy paired client and the frame loop measured through both. The README records the numbers and the thresholds, and a pass that misses the target is a design finding, not a tuning note.
- **Content versions.** The signals of §4.4 against the cases that defeat weaker ones: a same-size in-place rewrite of a pinned file with mtime restored by `touch -r`, a replace-by-rename, a run log truncated and regrown to the same length, a transcript appended within one poll interval; each must move the version. The two-second `stat` poll over 128 keys timed.
- Bonjour advertisement from the Rust process on macOS, visible to a visionOS simulator; the crate or system path that works.
- `URLSession` and `URLSessionWebSocketTask` on visionOS accepting the self-signed certificate through a pinning delegate that checks the SPKI hash and nothing weaker; the same delegate refusing a substituted certificate.
- The `openat` walk of §4.3 through `rustix` or `cap-std`: a pinned FIFO with no writer refused at once on its connection thread while the frame loop and a second connection keep going; a parent directory swapped for a symlink mid-walk refused.
- Two instances on one machine (`SWITCHBOARD_DATA_DIR`, `SWITCHBOARD_TMUX_SOCKET=switchboard-test-*`, different ports) each advertising with its own key.

Loopback-only and no auth are fine here and nowhere else.

### Milestone A — Host port and multi-host client foundation

Five commits, each leaving the app running and each with its README and design.md status update. The first user-visible result is a two-host read-only client at A2; nothing waits for the synchronization machinery.

**A0 — The conversation cache in the app.** `UiState.conversations` moves to `SwitchboardApp` as the first content cache, keyed by record id, resume handle and content version; the session, card and workflow views read it through a reference. No port code. Acceptance: the UI tests pass unchanged and the app behaves identically.

**A1 — Identity and an authenticated read-only host.** `identity.json` (id and certificate), the key in the Keychain under its installation's account, `server_name` in Settings, the remote port in Settings (off by default, loopback until enabled), `SWITCHBOARD_REMOTE_PORT`. `ports::remote`, `adapters/remote.rs` with the bounds of §4.1 and the two-step content handoff, a fake, the `Effect` and `AppAction` arms, the allowlist, the redaction function. The pairing window and QR code in the settings menu, the paired-devices dialog with scope and Revoke, `devices.json`. Endpoints: `/info`, `/pair`, projects, spaces, sets, sessions, session, waiting, screen. No events yet; a reply carries `epoch` and `revision` from the counter, which only the diff of A3 will move. Tests: `tests/remote.rs` against a real socket with fakes, modelled on `tests/control.rs`, covering pairing, the pin check, scope, revocation, the allowlist and screen redaction; a UI test for the dialog. Acceptance: a paired `curl` with the pinned certificate reads every view over the LAN; an unpaired one gets `/info` and nothing else.

**A2 — Two hosts and the first client.** Bonjour advertisement and the Linux stub. The Swift protocol, transport with the pinning delegate, `ServerRegistry`, `ConnectionManager`, and a read-only browser of sessions and sets with source labels, refreshing on a timer. Acceptance: two installations have distinct `server_id`s and keys; Bonjour discovers both; the client pairs with both and connects simultaneously; nothing mixes identifiers; taking one offline leaves the other working and its items visibly stale; a changed IP reconnects under the same pin.

**A3 — Synchronization.** The read-model diff after every pass, the revision counter moving, `changed` and `removed` events for metadata and collections, the WebSocket with `hello`, subscribe-then-snapshot and reconciliation by revision on the client, epoch handling, `resync_required`. `ops` and the log-before-run path in `serve` (no commands yet, but the path is tested here). Acceptance: the timer refresh of A2 is gone and every host-side change reaches the client within a pass; the mutation-between-hello-and-snapshot test passes on both sides.

**A4 — Watched content.** The watch set and its worker poll, `watch` and `unwatch` with `watching`, content versions for screen (the other content kinds arrive with Milestone C and use the same cache), eviction, the policy fingerprint in the cache key, the overflow close. Acceptance: a watched pane's screen updates live while the host shows an unrelated view or is hidden; the abuse tests of §4.1 pass against the real adapter.

Across A1 to A4: README (Layout, dev aids, data directory) and the design.md status section, with the trust boundary's sentence on redacted text added at A1.

### Milestone B — Native visionOS browser

- Grid, list and focus browsing of real sessions and sets from two hosts.
- Two items from different servers open as independent spatial windows.
- Each window offers `compact` and `screen`; the other modes arrive with Milestone C and the window says so until then.
- Closing a window never ends a session.
- Reconnect preserves open windows and refreshes them through subscribe-then-snapshot.

### Milestone C — Interactive sessions

- `session.conversation`, `session.last_response`, `session.runs`, `session.run_output` and `file.read` on the wire and the port, with `read_pinned` in the files adapter, `read_masked` on the transcript reader and the overlap read in `scrollback`.
- Conversation paging by turn, with the body cap, the oversized turn, the in-progress last turn and the rebind handled.
- `send`, `resume` and `trust` under `control` scope, with the operation id as the idempotency key and `/ops/{op}` on timeout.
- Live updates over the WebSocket through the watch set; scroll and disclosure restoration across incremental refetches and reconnects.
- System dictation into the text field as the initial voice input.

### Milestone D — Shared iPadOS shell

- The same modules and components on iPad with platform-appropriate navigation and windows.

## 11. Tests and failure cases

Host, in `tests/remote.rs` and the adapter's own tests unless said otherwise:

- Two hosts produce the same `RecordId` (seeded in fakes): no collision in UI or routing (client).
- A host's IP changes; Bonjour rediscovers it under the same pinned key (client, spike).
- Two advertisements claim one `server_id` with different keys: no automatic trust or merge (client).
- A substituted certificate during first pairing: the client refuses to send the code; and a `/pair` carrying another host's `spki_sha256` is refused by the host.
- Three wrong codes from three concurrent connections close the one pairing window; a fourth, correct, is refused. A consumed code is refused on the next `/pair`. Twenty unauthenticated connections and a `/pair` flood past the per-minute cap: a paired client's requests are answered throughout, the flood gets 429 and closed sockets.
- After Revoke, the device's next request fails and its WebSocket is closed. After a key rotation, `devices.json` is empty, every old token fails, and the dialog says the pairings are gone; the client, seeing the new key under a known `server_id`, sends nothing and asks to pair again.
- A scope lowered in the dialog refuses the device's next `send`; raised, allows it; no request over the port changes a scope.
- A mutation dispatched between a subscriber's `hello` and its first snapshot, and another during a multi-resource sync: the client ends with the host's state (host test on the ordering, client test on the reconciliation).
- An older snapshot reply arriving after a newer one is dropped by revision (client).
- The host restarts: the new `epoch` makes the client refetch everything.
- A `send` whose process is killed after the keys went into the pane and before the reply line (the fake host records the write, the fake log fails the reply append): the retry with the same op is answered `interrupted` and nothing is sent twice. A failed request-line append refuses the send and sends nothing. A dropped HTTP response after a logged reply is answered from the log.
- `unknown` for an op that never reached the frame; a resubmit runs once.
- The configured secret appears in a prompt, a tool result, a final response, a run's output, and a pinned non-dotenv text file: every content endpoint returns it masked. A secret store that fails makes every content endpoint refuse rather than answer.
- The configured secret straddles each cut: the parser's `DETAIL_CAP` inside a tool result, the 256 KiB turn cut, the first byte of a run-output tail window, and the front cut of a last response: each reply masks it whole or drops it whole, never a fragment.
- A project with three hundred sessions, each with 64 KiB notes and a 100 KiB `name`, listed whole: one reply under the cap, notes cut at 4 KiB and names at 4 KiB with each field in `truncated`, no member lost; the same session read alone carries its notes whole. A session entering and leaving `waiting` emits `changed {waiting}` and the refetched collection lacks or holds it accordingly; a member missing from a whole reply at a later revision is dropped by the client, a member missing from an older reply arriving late is not.
- A 700 KiB file of quotation marks is under the source limit and answers `too_large` from the serialized check; a pane with 200 lines of 4 KiB each is cut by bytes after masking and marked. A pinned file over the source limit answers `too_large`; a last response over the body cap is cut from the front and marked.
- Content versions (F2): a pinned file rewritten in place at the same size with its mtime restored, a file replaced by rename, a run log truncated and regrown to its old length, and a transcript appended twice within one poll interval: each read after the change carries new content and a new revision, and no two different bodies for one key share a revision within an epoch. Two reads of an unchanged file carry the same revision.
- Frame cost (F4): a 20 MiB transcript, a 768 KiB file and a Keychain read made to block for two seconds, each requested while a second connection reads session metadata: the metadata reply arrives within a pass and the frame's time per pass stays under its budget; only the requesting connection waits. A content handle read on the frame for a record removed before the worker finishes: the worker's version action is a no-op and the reply names the handle it read.
- A window on the last response alone, and one on the conversation, each refresh when the transcript grows with no session metadata change; a handle rebind refreshes both.
- With every content cache populated, a new environment-set grant, a changed secret value, and a secret store that starts failing: unchanged screen, file and run-output sources are not served from their old masked bytes; the first two emit `changed` for the watched keys and serve newly masked text, the third refuses.
- A watched session's transcript grows and a watched pinned file is edited while the host shows an unrelated project, a document, the Dispatch page, and while the window is hidden: `changed` arrives within the poll interval.
- A space rename, a session moved between projects, a project and a session removed, a time-based rule set losing a member: the right `changed` and `removed` events, no reconnect.
- A set renamed and a pin moved (saved through `SaveViews`), a Claude Code pane reaching its trust question (`PromptSeen`, no save), and a Codex pane passing its quiet interval (`HostListed`, no save, no screen change): each emits `changed` for the right resource within the pass. Ten passes over a running pane with nothing else moving emit nothing, though `quiet_secs` moved on each.
- Two reads of a watched file, a transcript and a pane separated by an external edit with no poll between them, delivered to the client in reverse order: the client ends on the newer content. A read of a session's metadata between them does not invalidate the conversation page.
- Closing one of two windows on a session sends no `unwatch`; closing the second does. A hundred and twenty-nine watched keys against the cap of 128: the least recently read is evicted, a read of it still answers with the right version and re-enters it, and no watch is refused. Reconnect resends the watches before any snapshot.
- Forget server on the client removes its token, pin, memory and views; a revocation seen on a connection drops the token and memory and keeps the views marked; nothing from a host is found on the client's disk after a relaunch (client).
- The subscriber queue overflows: `resync_required`, the socket closes, the host's frame loop and a second subscriber are unaffected (spike and adapter test).
- 20 stalled unauthenticated connections: the port stays answerable to a paired client (spike).
- `read_pinned`: a pin with `..`, an absolute path, an empty component; a symlink as a parent component (inside or outside the root, both refused) and as the leaf; a parent directory swapped for a symlink between two hops of the walk, driven from a second thread; a file that grows past the limit during the read; a FIFO with no writer, asserting the refusal returns at once and an unrelated request completes meanwhile; a directory and a device as the leaf. Each refused or capped as §4.3 says.
- A long transcript pages by turn; one enormous turn is cut and marked; a conversation whose handle changed answers with the new `resume_id` and the client refetches from 1 (client).
- An exited command whose pane is gone: `screen` refuses, `runs` and `run_output` answer from the log; a pruned log answers with the reason.
- Several windows show one session with independent view state and consistent data (client).
- A request outside the allowlist (`env.resolve`, `project.add`, a path that no set pins, a pinned `.env`) is refused with no side effect, and the refusal is logged by name only.
- Revocation while connected: the WebSocket closes, the next request fails with the reason, cached content stays readable on the client but is marked.
- A restart and a rebuild under the stable signing identity keep the key readable; two data directories under one macOS account keep separate keys; a missing key makes a fresh identity and the dialog says the pairings are gone.
- A `cfg(not(target_os = "macos"))` build compiles with the Bonjour adapter stubbed.

## 12. Answers to the first draft's questions

1. **Models and ids.** `src/core/model.rs`: `SessionRecord` (`RecordId`), `Project` (`ProjectId`), `WorkingSet` (`SetId`) with `PinnedItem { target: PinTarget, rect: GridRect }`, `CardState`, `Run` (a command's or service's run with its scrollback log name). Conversations are `ports::transcript::Conversation` with `Turn { n, user, activity, final_text, .. }`, read by `adapters/transcript.rs` from Claude Code's JSONL.
2. **Control port reuse.** Yes, directly: `app.rs::serve` takes a `wire::Request` and returns a `wire::Reply`; `core/control.rs` maps bodies to `ControlAction`. The remote adapter is another producer of requests for the same path, with auth, the allowlist and redaction in front, and the log-before-run step added to `serve` for non-replayable commands.
3. **Runtime.** None to own; the frame thread is the executor and `listen`/`serve_pending` are the startup and polling hooks. Shutdown drops the listener as the control socket does.
4. **Events.** No bus, and no effect that reliably marks a visible change: `Save`, `SaveViews` and `SaveSettings` are three effects, and `PromptSeen` and the quiet interval in `reconcile.rs` change a card with none of them. So the port diffs the exported read models after every pass, with the clock-derived `quiet_secs` held out of the hash, and versions content where it is read (§4.4). Retained replay is not needed: a subscriber is registered on the frame thread, so subscribe-then-snapshot has no gap.
5. **Pinned file access.** The document preview (`ui::document::Preview::load`) is a UI loader: a metadata check and then an unbounded read, which is fine for the user's own click and not for a remote reader. The port gets `read_pinned` in the files adapter: a descriptor-anchored `openat` walk with no symlink followed and a nonblocking open, as §4.3 says, run on the connection thread.
6. **Per-session UI state and polling.** `UiState` (drafts, scroll, folds) is transient and never read by the core; none of it crosses to the client. The conversation cache in `UiState` and the caption poll in `refresh_captions` both follow the local view: a session not on the Mac's screen is not polled, and a document, workflow, Dispatch or ticket view polls nothing. The app also parses a transcript synchronously on the frame (`refresh_conversation`), which one frame absorbs for one session on screen and would not for every watched session. The port therefore needs its own watch set, the cache lifted to the app, and content work on the connection and worker threads, which are the app-level changes the implementation constraint allows.
7. **Two installations.** `SWITCHBOARD_DATA_DIR` and `SWITCHBOARD_TMUX_SOCKET` on a `switchboard-test-*` socket; only the lock holder listens; `tests/control.rs` and `tests/gate.rs` show the pattern.

**Implementation constraint:** no change to the egui frontend beyond A0's cache move, and no change to how the transcript parser reads or shapes a conversation. The host work is one adapter, additive wire views, a `read_pinned` method on the files port, an additive `read_masked` on the transcript reader (the mask runs inside the existing parse, before its detail cap) and an overlap read in `scrollback`, the content caches, read-model diff, two-step content handoff and watch worker in `app.rs`, the log-before-run step in `serve`, a Settings field or two, and one dialog. Anything larger is proposed separately.

## 13. Explicit design decisions

| Decision | v1 choice |
|---|---|
| Topology | Independent hosts, no coordinator |
| Multi-host | Concurrent connections and unified browsing |
| Resources | Sessions, projects, working sets with pins, pinned files, runs; no card resource |
| Identity | `identity.json` UUID and certificate; key in the Keychain per installation; clients pin the public key |
| Discovery | Bonjour and manual endpoints |
| Host runtime | Frame-thread dispatch for core state; content work on connection and worker threads; blocking std server with bounded queues; no Tokio |
| Contract | The `switchboard-control` wire views, extended additively, behind an allowlist |
| Data fetch | HTTPS JSON, every reply stamped with epoch and revision |
| Live changes | One WSS per host; read-model diff after every pass, content versions assigned on read, watch worker; `changed`/`removed` events; refetch |
| Synchronization | Subscribe, then snapshot, then reconcile by revision; full refetch on a new epoch |
| Idempotency | The control port's operation id and `operations.log`, non-replayable commands logged before they run |
| Redaction | One policy over every text leaving the port, run on whole source text before any cut; fail closed |
| Caps | Collections answered whole with every string bounded; only the conversation pages; scalar content cut after masking or refused whole; the limit enforced on serialized bytes |
| Content versions | Transcript: size and mtime (append-only); file and run log: inode, ctime and size; screen: hash; no two bodies share a revision for one key |
| Masked caches | Versioned by source and by the project's redaction policy fingerprint; a resolution failure refuses, never serves a cached success |
| Watch set | Global cap of 128, least recently read evicted, never refused; a read always versions correctly |
| File reads | Descriptor-anchored `openat` walk, no symlink followed, nonblocking open, 768 KiB, on the connection thread |
| Authentication | Pairing window with QR, key verified before the code is sent, 40-bit single-use code with one global attempt counter, 256-bit bearer token without expiry, scope set in the host dialog; Keychain on both ends; key rotation clears every pairing |
| Client storage | Server list and presentation state on disk, token and pin in the Keychain, every host body in memory only |
| Milestone A | Five commits: cache move, authenticated read-only host, two-host client, synchronization, watched content |
| Session execution | Always on the owning host |
| Resume | User-initiated only, confirmed on the client |
| Presentation state | Client-owned |
| visionOS layout | Native movable windows |
| Remote internet access | Deferred; protocol-compatible |
| iPadOS | Shared Swift core and views |
