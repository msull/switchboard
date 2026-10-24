Every point I have left is wording or small tidiness; none blocks the round.

Style/Architecture review of 6909b3d (one commit since 7fe8949). The layering holds: `invite_url` is a pure function in `src/core/remote.rs`, so no logic moved under `ui/` and no I/O or clock reached the core. The docs are current: README Layout rows and the spike row, a dated design.md paragraph plus its known gap, CHANGELOG, and the corrected excerpt line in remote-client.md.

1. `src/app.rs` `authenticate`: `by_cookie = request.bearer.is_none()` is true even when the request carries no cookie. It only works because that case is refused two lines later. Name it for what it means, or compute it after the token is resolved, so the `Origin` rule reads "authenticated by cookie" without the reader re-deriving the fall-through. `request.bearer.clone().or_else(cookie)` also clones the bearer on every request to keep one `String` type, and the `cookie` closure shadows the idea of a cookie value. Wording-level: rename it `from_cookie`.

2. `src/app.rs` `/pair`: building the reply as a `json!` object and then `map.remove("token")` behind `if cookie && let serde_json::Value::Object(map) = &mut reply` is cleverer than it needs to be. The `Object` match can never fail. Build the body once without `token`, and insert it only for a native client, or `if !cookie { reply["token"] = ... }`. Simple code, and the "omits token" rule sits in one visible branch.

3. `src/adapters/remote.rs`: `respond_page` builds its header string with `fold` and `let _ = write!`, while `respond` does the same job with a `for` loop. Pick the loop in both. Also `write_response` takes five loose `&str` arguments, which is easy to transpose (`extra` and `connection` are both `&str`). It has no doc comment, and the file documents its other private helpers. A one-line comment saying `extra` is pre-formatted `Name: value\r\n` lines would do.

4. `src/adapters/remote.rs`: `Limits::over` is a new name beside the existing `limited`, and its doc is missing. Say that `limited` picks the window by path and `over` spends one request from a given window. `serve_page`'s doc is fine, but the `true`-means-close return is a bool with a meaning that has to be remembered. An enum is overkill, so keep the doc and move on.

5. `src/app.rs`: the `#[allow(clippy::too_many_lines)] // one arm per route` on `serve_remote` is acceptable, but it is the only such allow near these routes. If the lint fired only because of the one `me` arm, say why the function is now over the line, or pull the `me` arm into a `match` helper. Wording only.

6. `src/ui/remote.rs`: the RemoteDraft `qr` doc was updated correctly. The long help sentence is inline in `show`, as the neighbouring redaction sentence is, so I leave it. In `pairing`, `address` is built as a `String` and later checked with `.is_empty()`, an existing pattern. `ip.map(...).unwrap_or_default()` was kept, which is fine.

7. `README.md` dev-aid paragraph: the reflow left `opening the link in a browser pairs the browser. Script lines \`remote-listen lan <port>\` and` as one long line among 70-column lines. Rewrap it so the next diff is small.

8. `web/app.js`: nothing that narrates history. Comments say why (iPad-as-Mac, the 401 inference). I did not find debris. The pure parts are separated and tested as the plan asked.

No withdrawn points; this is the first round.
