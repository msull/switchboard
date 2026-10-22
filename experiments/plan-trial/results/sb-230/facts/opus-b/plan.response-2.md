# Response to the plan feedback for #230, round 2

All edits are in `review/1/repo/plan.md`. The earlier plan is untouched.

## Earlier points

1–5 and 7: settled; no change.

6. The accept-loop citation in decision 4 now reads `:701-705`.
   Section: Decisions, 4.

## Cuts

### Decisions

- **Decision 4, two ways to serve the files.** Cut. `serve_remote`
  now serves all three files on the frame, before the `/api/v1/`
  strip, with `authenticated` set when the cookie is a known token. The
  "without/with a cookie" paragraph is gone. The `page` window is still
  checked on the connection thread and bounds the frame's load, which
  decision 4 now says. Supporting changes:
  - The Signals bullet on `connection` is replaced by one on
    `Limits::limited`.
  - The `serve_remote` bullet under The actions and effects lost "that
    reached it (they carried a cookie)".
  - The §4.1 docs line now says "answered on the frame".
  - The README docs line moves the static files to the `src/app.rs`
    line (:401).

  One small addition: `RemoteReply` gains `content_type` so the frame's
  answer can carry `text/html` and the others. The plan had served
  these files from the connection thread, which wrote its own headers,
  so the frame needs it now. It is under Signals and in the Adapter
  contract list.
- **Decision 9 and the `web` capability.** Cut from Decisions, from
  the `remote_info` bullet under The actions and effects, from the
  `info_is_served_to_anyone…` test change, and from Contract additions.

### The record

- **`ServedCertificate::names`.** Cut from the struct, the doc
  comment, and the contract list. The departure bullet now says
  "reissues it only when its address no longer matches".
- **The list of `Identity` struct literals.** Cut.
- **The Rehydration paragraph.** Cut. Only the Known-gaps sentence
  about a changed address remains.

### The signals

- **"The existing constructors gain `cookie: None`".** Cut.

### The tests

- **"A read-only core emits none".** Cut from the Core
  `ServedCertificateMade` test.
- **"One with `served` round-trips".** Cut. "Loads without `served`"
  stays.
- **"The existing bearer tests and `stalled_strangers_…` pass
  unchanged".** Cut. The whole bullet is gone.
- **`tests/ui.rs`, "and `served: None` elsewhere".** Cut.

### The UI

- **Rereading `/device` on `visibilitychange`.** Cut. It is reread on
  `hello` and on `403 scope`, and the `visibilitychange` reconnect
  under Live data stays.
