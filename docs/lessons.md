# Lessons from Prompt Box

Prompt Box was the first app built from the original scaffold: a voice
dictation tool on top of whisper.cpp, egui, and OpenAI, built over two days
with an AI coding agent. These are the things that made it go well, in the
order they mattered. The template bakes in the ones that can be baked in.

## Architecture

**Put a deterministic core behind an action/effect boundary from day one.**
The scaffold started with logic as plain methods on the app struct. The
first real feature (streaming speech with provisional text) made it clear
that a state machine with explicit inputs (`AppAction`), explicit outputs
(`Effect`), and an injected `Clock` was the only way to test timing
behavior without sleeping. Every later feature slotted into that shape:
voice commands, AI rewrites on worker threads, tool plugins, toasts. Around
ninety core tests ran in well under a second. The template ships this
shape with one trivial feature in it so the pattern is there to copy.

**Ports and adapters earn their keep on the second adapter.** Clipboard,
file store, speech engine, typist, rewriter: each got a trait, a real
implementation, and a fake. The fakes made headless UI tests possible on
CI and let the whole app run in a "Debug" mode with scripted dictation and
no model. Write the fake at the same time as the real adapter, in the same
file.

**Keep `ui.rs` dumb.** Edit diffing and keyboard shortcuts were the only
logic that had to live near egui, and even those produce actions. When a
bug showed up, it was almost always reproducible as a core test.

## Process

**Spike risky technology in a throwaway crate first.** Before Milestone 1,
a standalone `spikes/voice-spike` crate answered whether whisper.cpp could
stream on this machine, what the real API looked like, and how long the
first Metal shader compile takes. Its README recorded the findings. The
spike's fixtures were reused as test inputs later.

**Milestones, each a working app.** Shell, then microphone, then editing
operations, then voice commands, then AI, then projects. Each milestone was
one commit with the README updated. Scope stayed honest because the app
had to run at every step.

**Write the design doc, then stop consulting it.** A product design doc
set priorities ("never silently lose speech" first). It was useful for
ordering milestones and for the layering diagram in `lib.rs`. Beyond that
the code and README became the source of truth, and the doc was left as
history.

**Pre-commit hook plus CI, both running the same three commands.** Nothing
formatted wrong, lint-dirty, or failing ever landed. The hook never modifies
files; it just says no. Clippy pedantic caught real mistakes and taught
idioms along the way. The two `allow`s in `Cargo.toml` are the only lints
that were pure noise.

**Audit comments once features settle.** Halfway through, a pass removed
comments that restated the code or narrated which milestone added what.
The rule since: comments say why.

## Tooling facts that cost time

- egui 0.36 renamed the `App` trait method to `ui`, merged panels into one
  `Panel` type, and kittest needs `.focus()` before `type_text`. See
  `CLAUDE.md`. When docs and compiler disagree, read the crate source in
  `~/.cargo/registry`.
- The crate forbids `unsafe`, and that survived native AppKit calls
  (Dock badge) because `objc2` exposes safe wrappers for the common
  methods. Put such code in a tiny `cfg`-gated adapter.
- macOS ties the Accessibility grant to the code signature, so an ad-hoc
  signed app loses the grant on every rebuild. A self-signed "Code Signing"
  certificate fixes it. The bundle script and README in Prompt Box have the
  recipe.
- A bundled `.app` starts with `/` as its working directory, so reading a
  `.env` from the current directory silently fails. Read secrets from a
  settings file in the platform data directory instead, with `.env` as a
  dev-only fallback.
- Two monitors: `screencapture -x a.png b.png` captures both; one path
  only gives the primary display.
- Linux CI for eframe needs `libxkbcommon-dev libgtk-3-dev libssl-dev`;
  audio adds `libasound2-dev`, whisper adds `cmake`.
