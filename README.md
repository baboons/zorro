# Zorro

**A modern Git merge conflict resolution tool for macOS.**

Zorro is a native macOS application focused exclusively on resolving Git merge
conflicts. Inspired by the JetBrains merge tool, it provides a fast,
keyboard-driven, developer-friendly interface built on the [Zed](https://zed.dev)
graphics engine (GPUI).

> Resolving merge conflicts should be fast, visual, and safe.

---

## Status

Early development. What works today:

- **`zorro-core`** — the headless conflict engine, fully tested:
  - Parses both `merge` and `diff3`/`zdiff3` conflict marker styles.
  - Models per-conflict resolutions: accept current / incoming / base / both /
    manual edit.
  - Renders resolved output, preserving CRLF/LF line endings and trailing
    newlines; a fully-unresolved document round-trips byte-for-byte.
  - Word- and character-level token diffing for in-conflict highlighting.
  - Discovers conflicted files and the active workflow (merge, rebase,
    cherry-pick, revert) via the `git` CLI.
- **`zorro`** — the GPUI macOS app:
  - Sidebar file list with per-file resolved/total counts.
  - JetBrains-style **full-height** three-column merge: **Local · Result ·
    Incoming**, row-aligned and **syntax-highlighted** (Rust, TS/JS, Go, Python,
    C#, Java, JSON, YAML). The Result column shows the **whole file** — unchanged
    context plus every conflict — with line numbers.
  - The Result's conflict regions are live, editable multiline code editors:
    cursor, typing, Enter/Backspace/Delete, arrows, Home/End, click-to-place,
    click-drag and shift-arrow **selection**, Select-All, and **copy/cut/paste**.
  - **Per-line diff colouring** inside each conflict: lines only on the left are
    red (removed), lines only on the right are green (added).
  - Gutters between the columns carry per-hunk accept (`»`/`«`) and ignore (`✕`)
    actions. Resolving a hunk **clears the rejected side** and swaps the gutter
    to an undo (`↺`), so done hunks read as done.
  - A bottom action bar with global **« Accept Left / Accept Right » / Reset /
    Save**. Each conflict is flagged amber (pending) until resolved, then green.
  - **Structural validation**: a resolved file whose brackets don't balance is
    marked **red** in the sidebar and save is blocked until it's fixed.
  - **diff3 offer**: repos using standard markers get a banner offering to switch
    to diff3 (regenerates the current conflicts with the common ancestor).
  - **AI assist**: `✦ Resolve all with AI` resolves every conflict in the file
    (provider calls run concurrently; each result is applied inline, with live
    progress, and is undoable + not saved until you Save). `✦ Explain` describes
    the focused conflict.
  - Keyboard-first navigation and resolution; writes resolved files to disk.

### AI-assisted resolution

`zorro-core::ai` is a provider-agnostic layer: it builds the prompt (base ·
current · incoming · surrounding context · path · language), parses the
response (fenced code + a confidence marker), and estimates confidence
(import-only / tiny → High, small → Medium, large → Low) as a fallback.

Providers implement `AiProvider`. The default is **Claude Code** (`claude -p`,
prompt piped over stdin); `CliProvider::codex()` and `CliProvider::ollama(model)`
are ready to use too. CLI providers run locally — no source leaves the machine
beyond what the chosen tool sends.

See [`SPEC.md`](SPEC.md) for the full product vision and roadmap.

---

## Building

GPUI is consumed from the Zed monorepo and requires the Rust toolchain Zed
pins. That version is captured in [`rust-toolchain.toml`](rust-toolchain.toml)
(currently **1.95.0**) and `rustup` will install it automatically.

```bash
# Run the headless engine tests (fast, no GPUI):
cargo test -p zorro-core

# Build and run the macOS app (first GPUI build takes a while):
cargo run -p zorro            # discovers conflicts in the current repo
cargo run -p zorro -- /path/to/repo
```

The app discovers the conflicted files in the repository at the current
directory (or the path passed as the first argument) and opens a window. With no
repository or no conflicts, it shows a friendly empty state.

---

## Keyboard shortcuts

| Action            | Shortcut |
|-------------------|----------|
| Next conflict     | <kbd>F7</kbd> |
| Previous conflict | <kbd>⇧F7</kbd> |
| Accept Current    | <kbd>⌥←</kbd> |
| Accept Incoming   | <kbd>⌥→</kbd> |
| Accept Both       | <kbd>⌥↓</kbd> |
| Next file         | <kbd>⌘↓</kbd> |
| Previous file     | <kbd>⌘↑</kbd> |
| Save file         | <kbd>⌘S</kbd> |

Click into a **Result** editor to edit the merged text directly. While it holds
focus: arrows, <kbd>Home</kbd>/<kbd>End</kbd>, <kbd>⏎</kbd>, <kbd>⌫</kbd>,
<kbd>⌦</kbd>; <kbd>⇧</kbd>+arrows or click-drag to select, <kbd>⌘A</kbd> select
all, <kbd>⌘C</kbd>/<kbd>⌘X</kbd>/<kbd>⌘V</kbd> copy/cut/paste.

---

## Architecture

```
zorro/
├── crates/
│   ├── zorro-core/   # headless engine: conflict parsing, diff, session, git
│   │                 # (no UI, no third-party dependencies)
│   └── zorro/        # GPUI macOS app (depends on zorro-core)
└── rust-toolchain.toml
```

The split keeps all conflict logic in a fast, dependency-free, fully-tested
library; the GPUI crate is purely presentation and input.

---

## License

MIT OR Apache-2.0
