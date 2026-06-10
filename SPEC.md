# Zorro — Product Specification

**A modern Git merge conflict resolution tool for macOS.**

Zorro is a native macOS application focused exclusively on resolving Git merge
conflicts. Inspired by the JetBrains merge tool experience, Zorro provides a
fast, keyboard-driven, developer-friendly interface built using the Zed graphics
engine.

## Vision

Resolving merge conflicts should be fast, visual, and safe.

Current tools are either:

- Embedded inside large IDEs
- Slow and memory-heavy
- Poorly adapted for modern Git workflows
- Difficult to use when conflicts span many files

Zorro aims to become the best standalone merge conflict resolver for developers
using Git.

---

## Goals

### Primary Goals

- Best-in-class merge conflict resolution experience
- Native macOS application
- Fast startup (<100ms target)
- Minimal memory usage
- Keyboard-first workflow
- Seamless Git integration

### Non-Goals

- Full IDE
- Source control client
- Code editor replacement

Zorro focuses only on conflict resolution.

---

## Core Features

### Three-Way Merge View

Display: Base · Incoming · Current · Result.

```
┌────────────┬────────────┐
│ Current    │ Incoming   │
├────────────┴────────────┤
│ Result                  │
└─────────────────────────┘
```

Alternative: `Current | Base | Incoming` over `Result`. Users can switch
layouts.

### Conflict Navigation

| Action | Shortcut |
|--------|----------|
| Next conflict | F7 |
| Previous conflict | Shift+F7 |
| Accept Left | ⌥← |
| Accept Right | ⌥→ |
| Accept Both | ⌥↓ |
| Manual Edit | Enter |

### Smart Conflict Actions

Per conflict: Accept Current, Accept Incoming, Accept Both, Swap Order, Edit
Manually. Inline buttons appear directly beside conflict markers.

### Syntax Highlighting

TypeScript, JavaScript, Rust, Go, Python, C#, Java, JSON, YAML, Markdown.
Leverage Zed's syntax infrastructure where possible.

### Diff Engine

Word-level diffs, character-level diffs, whitespace ignoring, moved block
detection, side-by-side comparison.

### Git Integration

Configure as `git config --global merge.tool zorro`. Supported workflows:
`git merge`, `git rebase`, `git cherry-pick`, `git stash pop`, `git apply`.
Zorro detects the workflow context automatically.

---

## Session Overview

```
Merge Session
─────────────
[ ] auth.ts
[✓] user.ts
[ ] payment.ts
[ ] checkout.ts

1 / 4 files completed
```

Users can jump between files instantly.

## File Tree

Sidebar with `Resolved`, `Unresolved`, `Modified`, `Binary` indicators.

## Search

Global search across current file, all conflicted files, resolved conflicts,
and Git paths.

## AI Assistance (Future)

Optional, disabled by default: explain conflict, suggest resolution, summarize
changes, detect semantic conflicts.

## Binary File Support

Handle images, PDFs, design assets. Actions: Keep Current / Keep Incoming.
Image files receive visual previews.

---

## Performance Targets

| Metric | Target |
|--------|--------|
| Startup | <100 ms |
| Open 100 conflicts | <1 sec |
| Memory | <200 MB |
| Conflict navigation | Instant |

## Design Principles

- **Native** — feels like a first-party macOS app.
- **Fast** — every interaction feels immediate.
- **Focused** — only solve merge conflicts.
- **Keyboard First** — mouse optional.
- **Safe** — never destroy data without confirmation.

## User Interface

Dark-first, light mode supported. Minimal, dense, professional developer
tooling. Influences: JetBrains Merge Tool, Zed, Xcode, Linear.

---

## Architecture

- **Frontend:** Rust + Zed UI framework (GPUI)
- **Core:** Git integration layer, diff engine, merge engine, session manager
- **Future:** Linux support, Windows support

## MVP (1.0)

Three-way merge · file navigation · Git integration · syntax highlighting ·
keyboard shortcuts · session overview · native macOS app. Everything else can be
added later.
