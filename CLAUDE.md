# collab-tui

A terminal-native collaborative markdown editor using CRDTs for conflict-free sync over SSH.

## Architecture

- **Approach**: Hybrid of spec Methods A and B — dumb SSH file server for storage, yrs CRDT for sync correctness, local-first editing
- **Server**: No daemon. Just a filesystem accessible via SSH (sshd + filesystem). Stores markdown files and yrs update deltas
- **Client**: Rust TUI application that edits locally, syncs via SSH push/pull of yrs deltas
- **Sync model**: Not real-time keystroke streaming. Sync on save or periodic polling. Users don't need to be online simultaneously. yrs guarantees convergence regardless of update ordering

## Tech Stack

- **Language**: Rust
- **TUI**: `ratatui` + `crossterm`
- **Text editing**: `tui-textarea`
- **Markdown parsing**: `pulldown-cmark`
- **Markdown rendering**: `termimad`
- **CRDT engine**: `yrs` (y-crdt)
- **Async runtime**: `tokio`
- **Filesystem watching**: `notify`
- **SSH transport**: `openssh` or shell out to `ssh`/`scp`
- **Directory traversal**: `walkdir` + `ignore`
- **Config/serialization**: `serde` + `toml`

## Layout

Three-pane TUI: file tree (left) | editor (center) | preview (right), with a status bar.

## Server Filesystem Convention

```
/srv/docs/
├── project-alpha/
│   ├── README.md
│   ├── notes.md
│   └── .yrs/
│       ├── README.bin          # full yrs doc state
│       ├── notes.bin
│       └── updates/            # append-only deltas
│           ├── 001_alice_<ts>
│           └── 002_bob_<ts>
└── project-beta/
    └── spec.md
```

## Development Guidelines

- Keep dependencies minimal — don't add crates unless clearly needed
- Prefer simple shell-based SSH (scp/ssh commands) over Rust SSH libraries for the initial implementation
- Test sync logic with two local yrs docs before involving SSH
- Periodic compaction: merge pending updates into base state to avoid unbounded growth
- The spec file `collab-tui-spec.md` contains the full design exploration — reference it for detailed diagrams and alternative approaches

## Build & Run

```bash
cargo build
cargo run
```
