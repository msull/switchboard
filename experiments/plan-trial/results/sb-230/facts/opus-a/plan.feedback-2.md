# Plan feedback, round 2

## Earlier points

1. Watch set: settled. Excerpts are capped at 8 rows, fetched on load and on that session's own `changed`, and the 9-key cost is stated with `:254` cited (plan lines 29-39, 353-370).
2. A 401 on the upgrade: settled. Any socket failure is followed by one `GET device`, which tells revoked (401), unreachable (network error) and a fallback failure (200) apart, and the page retries the socket once a minute while polling (lines 122-137, 375-380, 424-425).
3. The 404's `Connection` header: settled. `respond_closing` writes `Connection: close` for both the page files and the 404, and the test names the exact headers (lines 237-245, 399-400).
4. Refusal log: settled. The connection thread logs method and path only, the query is never logged (the path field excludes it, `src/adapters/remote.rs:362-363`, checked), and the §4.1 doc rule is added (lines 232-233, 246-250, 457-459).
5. CSP `'self'`/`wss`: settled. CSP3 §6.7.2.8 and WebKit bug 235873 are cited, the `wss://<Host>` fallback is limited to WebKit older than April 2022, and a bare `wss:` is ruled out (lines 265-275).
6. Citations and §5 lines: settled. The HTML standard's `WindowLocalStorage` IDL is cited, the case where `localStorage` throws `SecurityError` is handled, and `:309`, `:313` and `:317` are named (lines 174-182, 471-478).

## Cuts

### Where this departs from the issue
- Lines 40-42 ("If the owner wants every row excerpted, a later issue adds…") and lines 55-56 ("Recording revoked hashes would be new host state…"): work this cut does not do. An implementer doesn't need either.
- Lines 72-76, "Facts correction": the plan does not reissue the certificate, so nothing here is built on those lines.

### The record
- Lines 85-88, Decision 6's hypothetical `kind` field: describes a field that is not added. "No change; a browser is a `PairedDevice` named by its `device_name`" is enough.

### The signals
- A section with nothing to say. The one new input, the subprotocol token, is fully specified in Actions 2. Drop the section.

### The actions and effects
- 1, lines 107-114 (why not install the certificate) and 143-144 (the follow-up issue if `fetch` fails): rationale for a path not built.
- 2, lines 166-167: repeats facts `:13`/`:15` and the Commit section's "outside the contract".
- 2, lines 168-173, "Why not the cookie": rejected alternative.
- 2, lines 190-193, the optional `Origin` check "if the owner wants it": machinery for an option not taken.
- 3, lines 208-212, the `.local` alternative: keep at most "IP, because `.local` resolution is Unresolved"; the rest describes a change not made.
- 5, lines 300-302, the `/pair`-scope fallback: a second mechanism where `GET device`, which this plan builds, already suffices.
- 5, line 298, "Spec §4.3 lists it": repeated in Docs (line 460).

### The UI
- Lines 365-370: restate the 9-key account already given at lines 33-39. Keep it in one place.
- Lines 375-380: restate Actions 1's post-failure `GET device` rule. A pointer to Actions 1 is enough.

### The tests
- Line 401, "The existing `/other` check (`:465`) still passes", and lines 427-428, "The existing `subscribe_at` (`:1960`) still upgrades…": existing tests force both.
- Line 443, "Fixtures do not change": the fixture tests force it.

### Docs, Commit
- Nothing to cut.
