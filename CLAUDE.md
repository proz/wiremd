# wiremd

A terminal-native collaborative markdown editor using CRDTs for conflict-free sync over SSH.

## Architecture

- **Approach**: Hybrid — dumb SSH file server for storage, yrs CRDT for sync correctness, local-first editing
- **Server**: No daemon. Just a filesystem accessible via SSH (sshd + filesystem). Stores markdown files and yrs update deltas
- **Client**: Rust TUI application that edits locally, syncs via SSH push/pull of yrs deltas
sdfsdfsdfsdf - **Sync model**: Not real-time keystroke streaming. Sync on save or periodic polling. Users don't need to be online simultaneously. yrs guarantees convergence regardless of update ordering

## Tech Stack

- **Language**: Rust
- **TUI**: `ratatui` 0.29 + `crossterm` 0.28
- **Text editing**: `tui-textarea` 0.7
- **Markdown parsing**: `pulldown-cmark` 0.13
- **Markdown rendering**: Custom renderer (pulldown-cmark offset iter → ratatui Spans/Lines with source line mapping)
- **CRDT engine**: `yrs` (y-crdt) — planned
- **Async runtime**: `tokio` — planned
- **Filesystem watching**: `notify` — planned
- **SSH transport**: `openssh` or shell out to `ssh`/`scp` — planned
- **Directory traversal**: `walkdir` + `ignore` — planned
- **Config/serialization**: `serde` + `toml` — planned

## Current State

### Working
- Markdown viewer with rendered output (headings, code blocks, tables, lists, links, blockquotes, rules, task lists, inline code)
- Word wrapping at configurable MAX_WIDTH (80)
- Code blocks with uniform background and margin indentation
- Tables with box-drawing borders, aligned columns, bold headers, empty cell support
- View/edit toggle mode: rendered markdown view ↔ raw markdown editor
- Source line mapping: cursor position preserved when switching between view and edit modes
- Full-width cursor line highlight in view mode
- Scrollbar, vim-style navigation (j/k, g/G, space, page up/down)
- File save (Ctrl+S in edit, s in view)
- Modified indicator in title bar

### Not Yet Implemented
- Three-pane layout (file tree, editor, preview)
- yrs CRDT integration
- SSH sync
- User presence
- Config file support

## UI Modes

- **View mode**: Rendered markdown with cursor line highlight. Navigate with j/k, edit with e/Enter, save with s, quit with q/Esc
- **Edit mode**: Raw markdown in tui-textarea with line numbers. Save with Ctrl+S, return to view with Esc

## Development Guidelines

- Keep dependencies minimal — don't add crates unless clearly needed
- Pin ratatui to 0.29 to match tui-textarea 0.7 compatibility
- Prefer simple shell-based SSH (scp/ssh commands) over Rust SSH libraries for the initial implementation
- Test sync logic with two local yrs docs before involving SSH
- Periodic compaction: merge pending updates into base state to avoid unbounded growth
- The spec file `resources/collab-tui-spec.md` contains the full design exploration — reference it for detailed diagrams and alternative approaches

## Build & Run

```bash
cargo build
cargo run -- <file.md>
```
