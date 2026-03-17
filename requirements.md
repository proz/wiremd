# Requirements — wiremd

## Overview

A terminal-native collaborative markdown document editor. Users run a local TUI client that edits documents backed by yrs CRDTs. A shared SSH-accessible server stores the canonical files and sync deltas. No server daemon required.

---

## Functional Requirements

### F1 — TUI Interface

- F1.1: ~~Three-pane layout: file tree, editor, markdown preview~~ → Single-pane with view/edit toggle (markdown is readable enough that a separate preview is unnecessary)
- F1.2: File tree shows directory hierarchy from the configured document root (planned)
- F1.3: Editor supports markdown editing via tui-textarea
- F1.4: View mode renders markdown with styled output (headings, code blocks, tables, lists, etc.)
- F1.5: Status bar shows: connection state, last sync time, current file, active users (planned)
- F1.6: Keyboard-driven navigation with vim-style bindings
- F1.7: Cursor line highlight in view mode, cursor position preserved across mode switches
- F1.8: File tree indicates which files are being edited by other users (planned)

### F2 — Local Editing

- F2.1: All edits happen on a local yrs document — no network latency for typing
- F2.2: Changes are tracked as yrs CRDT operations
- F2.3: Undo/redo support via yrs undo manager
- F2.4: Local file cache so documents are available offline

### F3 — Sync via SSH

- F3.1: Push local yrs update deltas to the server on save
- F3.2: Pull pending yrs update deltas from the server on open and on demand
- F3.3: Sync uses standard SSH (scp/ssh/rsync) — no custom server software
- F3.4: Server filesystem stores: markdown files, yrs document state, and append-only update deltas
- F3.5: Update deltas are named with user ID and timestamp for ordering
- F3.6: Automatic or configurable sync interval (manual, on-save, or periodic polling)
- F3.7: Convergence guaranteed by yrs regardless of update arrival order

### F4 — Conflict Resolution

- F4.1: Concurrent edits to the same file merge automatically via yrs CRDT
- F4.2: No lock files, no last-write-wins — all edits are preserved
- F4.3: Users can see merged result after sync completes

### F5 — Compaction

- F5.1: Periodic compaction merges all pending update deltas into the base yrs document state
- F5.2: Compaction cleans up processed update files from the server
- F5.3: Compaction can be triggered manually or on a threshold (e.g., after N updates)

### F6 — User Presence

- F6.1: Track which users have a file open via presence metadata on the server
- F6.2: Display active editors in the file tree and status bar
- F6.3: Presence is best-effort — stale presence entries expire after a timeout

---

## Non-Functional Requirements

### NF1 — Performance

- NF1.1: Editing must feel instant — no perceptible latency from CRDT operations
- NF1.2: Markdown re-render on mode switch must be imperceptible
- NF1.3: Sync operations run in background, never block the UI
- NF1.4: Handle documents up to 100K lines without degradation

### NF2 — Portability

- NF2.1: Runs on Linux and macOS
- NF2.2: Works in any terminal emulator supporting 256 colors
- NF2.3: Usable over SSH sessions

### NF3 — Security

- NF3.1: All transport uses SSH — encryption and auth handled by SSH
- NF3.2: No custom auth system — relies on SSH keys / server access
- NF3.3: No sensitive data stored in plaintext config

### NF4 — Reliability

- NF4.1: Graceful handling of SSH connection failures — queue updates for retry
- NF4.2: No data loss on crash — local yrs state persisted to disk
- NF4.3: Corrupted update deltas are skipped with a warning, not fatal

---

## Configuration

```toml
# ~/.config/wiremd/config.toml

[server]
host = "docs.example.com"
user = "alice"
docs_path = "/srv/docs"

[local]
cache_dir = "~/.cache/wiremd"
sync_mode = "on-save"        # "manual" | "on-save" | "poll"
poll_interval_secs = 10

[editor]
tab_width = 4
line_numbers = true

[user]
name = "alice"
```

---

## Out of Scope (for now)

- Real-time keystroke-level sync (push-based WebSocket or P2P)
- Image/binary asset handling
- Mobile or web companion viewer
- Git integration
- Multi-cursor display within the editor pane
- User access control beyond SSH permissions
- Split view (editor + preview side by side)
