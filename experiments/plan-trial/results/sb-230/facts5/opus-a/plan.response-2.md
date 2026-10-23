# Response to plan feedback 2

## Earlier points

1–4 are settled. Nothing further changed for them.

## Cuts

- **"Scan the QR with a phone":** cut from **Where this departs from
  the issue**. Step 6.2 and the dialog hint under **The UI** still say
  "tap the link".
- **Rehydration:** cut from **The record**. Its one step the page
  sequence lacked was opening the socket and waiting for `hello` before
  fetching, then applying buffered events by revision. That step now
  opens page step 4 (**The UI**, List), so the load order is given once.
- **Decision 5:** cut the sentences explaining `include_str!`.
- **`RemoteReply` line numbers:** cut from **The actions and effects**.
  The bullet now says only that every existing literal takes
  `set_cookie: None`.
- **`info.capabilities` gaining `"device"`:** cut from **The actions
  and effects** (`app.rs`) and from **Contract additions**.
- **Swift `SessionView.lastMessage`:** cut from **The actions and
  effects** (the excerpt), from **Contract additions**, and from the
  check in step 5 of **The tests**. The Fixtures bullet in step 3 now
  says Swift's decoding ignores the unknown key, so the Swift fixture
  test passes unchanged.
