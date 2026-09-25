# Spikes

Throwaway crates and scripts that answered the Spike 0 questions in
`docs/design.md` before any product code was written. Each directory has a
README with the commands run, their output, and a recommendation. They are
kept for reference and are not built by the main crate.

| Dir | Question | Answer |
| --- | --- | --- |
| `01-claude-resume` | Can Claude Code sessions be captured and resumed? | Yes: `--session-id <uuid>` at launch, `--resume <uuid>` later from any terminal. Transcripts pruned after 30 days unless `cleanupPeriodDays` is raised. |
| `02-process-host` | tmux or own PTYs? | tmux on a private server: sessions outlive the app, reattach works, ~3 ms round trip. PTY adapter kept as fallback and for tests. |
| `03-session-state` | How to know "waiting on you"? | Claude Code hooks (`PermissionRequest`, `Stop`, `SessionEnd`) via a tiny `switchboard-hook` binary and a Unix socket; OSC 777 text from the raw stream as the hook-free fallback. |
| `05-keychain` | Can secrets live in the Keychain without prompts? | Yes for a bundle signed with a stable identity and identifier: the item ACL is `identifier + certificate`, so rebuilds read silently. Ad-hoc builds prompt per rebuild. |
| `06-pane-size` | Why is the embedded pane wider than the widget? | `window-size manual` ignored the client; `latest` makes the window follow whichever client attached last. Re-applied on launch for servers that outlive the app. |
| `07-clone-session` | Can a session be cloned up to a chosen message? | Yes: copy the transcript prefix before the chosen prompt, substitute a fresh UUID for `sessionId`, save it `0600` beside the original; `--resume <new>` opens the truncated history and the original is untouched. |
| `08-promptbox-embed` | Can Prompt Box be the agent message box? | Yes: same egui 0.36, drawn into a bottom panel unchanged, a `Clipboard` adapter turns Send into a hand-off; needs an embedded mode, a sink port, and focus-scoped shortcuts in promptbox. |
| `09-popout` | Can a session live in a window of its own? | Yes: an immediate viewport drawn from the main frame holds the page, terminal, and side panel; its frame is saved to settings and restored. |
| `10-nunchuk-serial` | Can a nunchuk drive the working set over USB serial? | Yes: one line per button change or stick flick from the Feather, read on a thread that reconnects; round trip 2 to 9 ms. Firmware in `firmware/nunchuk`. |
| `04-terminal-view` | Embedded terminal or hand-off? | Hand off agents to Ghostty (`open -na Ghostty --args ... -e cmd`, raise by title); embed `egui_term` for shells, commands, services. |

Findings are folded into `docs/design.md` under "Spike 0 results".
