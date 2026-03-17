# Collaborative TUI Document Editor — Concept & Architecture

**A terminal-native collaborative workspace for editing and browsing markdown documents in real-time.**

*Think: yazi (file browser) + glow (markdown renderer) + multiplayer editing — all in the terminal.*

---

## 1. The Problem

There is no integrated terminal-based tool that combines:

- A file browser for navigating a shared document tree
- A markdown editor with live rendered preview
- Real-time collaboration between multiple users
- Awareness of who is editing what

Existing solutions are either web-based (Etherpad, HedgeDoc), GUI-only (Obsidian, Typora), or require sharing an entire terminal session (tmux/tmate) with no document-level granularity.

---

## 2. Design Goals

- **Terminal-native**: runs in any modern terminal emulator, over SSH
- **Markdown-first**: `.md` files are the primary document format
- **Multi-user**: multiple people can browse, edit, and preview simultaneously
- **Lightweight**: no browser, no Electron, no GUI toolkit
- **Simple hosting**: should run on a basic Linux VPS with SSH access

---

## 3. Architectural Approaches

Three methods are explored below, from simplest to most sophisticated.

---

### Method A — SSH + Shared Filesystem (Simplest)

**Concept**: Each user SSHes into a shared server and runs a local TUI app that operates directly on the server's filesystem. No sync layer needed — the filesystem *is* the shared state.

```
┌─────────────────────────────────────────────────────────────┐
│                      REMOTE SERVER                          │
│                                                             │
│   ┌─────────────────────────────────────────────────────┐   │
│   │              Shared Filesystem                      │   │
│   │   /srv/docs/                                        │   │
│   │   ├── project-alpha/                                │   │
│   │   │   ├── README.md                                 │   │
│   │   │   ├── notes.md                                  │   │
│   │   │   └── meeting-2026-03.md                        │   │
│   │   └── project-beta/                                 │   │
│   │       └── spec.md                                   │   │
│   └─────────────────────────────────────────────────────┘   │
│          ▲              ▲               ▲                    │
│          │ read/write   │ read/write    │ read/write         │
│          │              │               │                    │
│   ┌──────┴──┐    ┌──────┴──┐     ┌──────┴──┐                │
│   │ TUI App │    │ TUI App │     │ TUI App │                │
│   │ (Alice) │    │  (Bob)  │     │ (Carol) │                │
│   └─────────┘    └─────────┘     └─────────┘                │
│       ▲               ▲               ▲                     │
└───────┼───────────────┼───────────────┼─────────────────────┘
        │ SSH           │ SSH           │ SSH
   ┌────┴────┐    ┌─────┴───┐    ┌─────┴───┐
   │ Alice's │    │  Bob's  │    │ Carol's │
   │ terminal│    │ terminal│    │ terminal│
   └─────────┘    └─────────┘    └─────────┘
```

**Change detection flow:**

```
  User A saves file
       │
       ▼
  inotify/fanotify detects change on server filesystem
       │
       ▼
  Notification broadcast to all connected TUI instances
  (via Unix socket, named pipe, or a lightweight daemon)
       │
       ▼
  Other TUI instances reload the file and refresh display
```

**Conflict handling:**

```
  User A opens spec.md         User B opens spec.md
       │                            │
       ▼                            ▼
  Lock file created:           Lock file detected:
  spec.md.lock (owner=alice)   ⚠ "alice is editing this file"
       │                            │
       ▼                            ▼
  A edits freely               B can: (1) view read-only
                                      (2) force-edit (last-write-wins)
                                      (3) wait for lock release
```

**Pros**: Dead simple. No sync protocol. Standard SSH. Server needs nothing beyond sshd and inotify.
**Cons**: No concurrent editing of the same file. Conflict resolution is crude. All users must SSH into the same machine.

**Existing tools that approximate this today:**

| Tool | Role |
|------|------|
| `sshd` | Transport — users connect to the shared server |
| `inotifywait` (inotify-tools) | Watch filesystem for changes |
| `helix` / `neovim` | Editor running on the server |
| `glow` | Markdown preview in a second pane |
| `tmux` / `zellij` | Multiplexer to split editor + preview + file tree |
| `yazi` / `lf` / `ranger` | File browser in the terminal |
| `entr` | Re-run glow preview on file change |

**Glue script (what you can do today):**

```bash
# On the server, each user runs:
tmux new-session -s workspace \; \
  split-window -h \; \
  send-keys 'yazi /srv/docs' Enter \; \
  select-pane -L \; \
  split-window -v \; \
  send-keys 'ls /srv/docs/**/*.md | entr -r glow /srv/docs/README.md' Enter \; \
  select-pane -U \; \
  send-keys 'hx /srv/docs/README.md' Enter
```

---

### Method B — Client-Server with WebSocket Sync

**Concept**: A dedicated server daemon manages documents and broadcasts changes over WebSockets. Each user runs a TUI client locally that connects to the server. Files live on the server but edits are transmitted as operations, not raw file writes.

```
                    ┌──────────────────────────────────────┐
                    │           SERVER DAEMON               │
                    │                                       │
                    │  ┌─────────┐    ┌──────────────────┐  │
                    │  │ Document│    │  User/Session     │  │
                    │  │ Store   │    │  Manager          │  │
                    │  │ (.md    │    │  - who's online   │  │
                    │  │  files) │    │  - who edits what │  │
                    │  └────┬────┘    │  - cursor pos     │  │
                    │       │         └──────────────────┘  │
                    │       │                               │
                    │  ┌────┴──────────────────────────┐    │
                    │  │  Operational Transform (OT)   │    │
                    │  │  or simple diff/patch engine   │    │
                    │  └────┬──────────────────────────┘    │
                    │       │                               │
                    │  ┌────┴──────────────────────────┐    │
                    │  │  WebSocket broadcast layer     │    │
                    │  └──┬──────────┬──────────┬──────┘    │
                    └─────┼──────────┼──────────┼───────────┘
                          │ wss://   │ wss://   │ wss://
                          │          │          │
                   ┌──────┴──┐ ┌─────┴───┐ ┌───┴─────┐
                   │ TUI     │ │ TUI     │ │ TUI     │
                   │ Client  │ │ Client  │ │ Client  │
                   │ (Alice) │ │ (Bob)   │ │ (Carol) │
                   │         │ │         │ │         │
                   │ ┌─────┐ │ │ ┌─────┐ │ │ ┌─────┐ │
                   │ │File │ │ │ │File │ │ │ │File │ │
                   │ │tree │ │ │ │tree │ │ │ │tree │ │
                   │ ├─────┤ │ │ ├─────┤ │ │ ├─────┤ │
                   │ │Edit │ │ │ │Edit │ │ │ │Edit │ │
                   │ │pane │ │ │ │pane │ │ │ │pane │ │
                   │ ├─────┤ │ │ ├─────┤ │ │ ├─────┤ │
                   │ │Prev.│ │ │ │Prev.│ │ │ │Prev.│ │
                   │ │pane │ │ │ │pane │ │ │ │pane │ │
                   │ └─────┘ │ │ └─────┘ │ │ └─────┘ │
                   └─────────┘ └─────────┘ └─────────┘
```

**Edit flow with operational transform:**

```
  Alice types "hello" at line 5, col 10
       │
       ▼
  TUI client creates operation:
  { type: "insert", path: "spec.md", pos: {line:5, col:10}, text: "hello" }
       │
       ▼
  Sent to server via WebSocket
       │
       ▼
  Server applies operation to canonical document
  Server transforms against any concurrent operations from Bob
       │
       ▼
  Server broadcasts transformed operation to all other clients
       │
       ▼
  Bob's TUI applies the transformed insert, screen updates
```

**Pros**: Real concurrent editing. Cursor awareness. Works across networks (not just SSH).
**Cons**: Requires building a server daemon. OT is complex to implement correctly. More infrastructure.

**Existing tools and libraries:**

| Component | Tool / Library | Language | Purpose |
|-----------|---------------|----------|---------|
| TUI framework | `ratatui` | Rust | Terminal UI rendering |
| Text editing widget | `tui-textarea` | Rust | Editable text area component |
| Markdown parsing | `pulldown-cmark` | Rust | Parse .md to AST for rendering |
| Markdown terminal rendering | `termimad` | Rust | Render markdown to styled terminal output |
| WebSocket client | `tokio-tungstenite` | Rust | Async WebSocket for client-server communication |
| WebSocket server | `axum` + `tokio` | Rust | Lightweight async server |
| Diff/patch | `similar` | Rust | Text diffing library |
| Syntax highlighting | `syntect` or `tree-sitter` | Rust | Code block highlighting in preview |

---

### Method C — Peer-to-Peer CRDT Sync (Most Sophisticated)

**Concept**: No central server required for editing. Each client maintains a local CRDT replica of the document. Changes are synchronized peer-to-peer (or via a lightweight relay). All replicas converge automatically, even after offline editing.

```
  ┌─────────────────────────────────────────────────────────────────┐
  │                    NETWORK TOPOLOGY                             │
  │                                                                 │
  │     ┌──────────────┐                    ┌──────────────┐        │
  │     │  Alice's TUI │◄──── P2P sync ────►│  Bob's TUI   │        │
  │     │              │    (iroh / QUIC)    │              │        │
  │     │ ┌──────────┐ │                    │ ┌──────────┐ │        │
  │     │ │ Local    │ │                    │ │ Local    │ │        │
  │     │ │ CRDT     │ │                    │ │ CRDT     │ │        │
  │     │ │ Replica  │ │                    │ │ Replica  │ │        │
  │     │ └──────────┘ │                    │ └──────────┘ │        │
  │     └──────┬───────┘                    └──────┬───────┘        │
  │            │                                   │                │
  │            │         P2P sync                  │                │
  │            │      ┌──────────────┐             │                │
  │            └─────►│  Carol's TUI │◄────────────┘                │
  │                   │              │                              │
  │                   │ ┌──────────┐ │                              │
  │                   │ │ Local    │ │                              │
  │                   │ │ CRDT     │ │                              │
  │                   │ │ Replica  │ │                              │
  │                   │ └──────────┘ │                              │
  │                   └──────────────┘                              │
  │                                                                 │
  │   Optional: lightweight relay/signaling server for NAT          │
  │   traversal and peer discovery                                  │
  └─────────────────────────────────────────────────────────────────┘
```

**CRDT merge example:**

```
  Initial state: "The cat sat"
  
  Alice (offline):  inserts "big " → "The big cat sat"
  Bob (offline):    inserts " down" → "The cat sat down"
  
  CRDT character IDs (simplified):
  
  Alice's view:     T  h  e  [b  i  g  ' '] c  a  t     s  a  t
  IDs:              1  2  3   A1 A2 A3 A4   4  5  6  7  8  9  10
  
  Bob's view:       T  h  e     c  a  t     s  a  t  [' ' d  o  w  n]
  IDs:              1  2  3  7  4  5  6  7  8  9  10  B1  B2 B3 B4 B5
  
  After sync — both converge to:
                    "The big cat sat down"
  
  Merge is automatic and deterministic because each character
  has a unique ID with a total ordering.
```

**Pros**: Works offline. No central server needed. True concurrent keystroke-level editing. Mathematical convergence guarantee.
**Cons**: Most complex to implement. Needs NAT traversal for direct P2P. CRDT document size can grow over time (tombstones).

**Existing tools and libraries:**

| Component | Tool / Library | Language | Purpose |
|-----------|---------------|----------|---------|
| CRDT engine | `y-crdt` (yrs) | Rust | Yjs port — mature sequence CRDT for text |
| CRDT engine (alt) | `automerge` | Rust | Alternative CRDT with JSON document model |
| P2P networking | `iroh` | Rust | Encrypted P2P connections with hole-punching |
| P2P networking (alt) | `libp2p` | Rust | Modular P2P networking stack |
| TUI framework | `ratatui` | Rust | Terminal rendering |
| Markdown parsing | `pulldown-cmark` | Rust | Markdown AST |
| File watching | `notify` | Rust | Cross-platform filesystem watcher |

---

## 4. TUI Layout Concept

All three methods share the same user-facing TUI layout:

```
┌─────────────────────────────────────────────────────────────────────┐
│  collab-edit ── /srv/docs/project-alpha     👤 alice, bob (2 online)│
├──────────────┬──────────────────────┬───────────────────────────────┤
│              │                      │                               │
│  FILE TREE   │   EDITOR             │   PREVIEW                    │
│              │                      │                               │
│  ▼ project-  │   # Meeting Notes    │   ┌─────────────────────┐    │
│    alpha/    │                      │   │                     │    │
│    README.md │   ## Attendees       │   │  Meeting Notes      │    │
│  ► notes.md  │                      │   │  ═════════════      │    │
│  ● meeting.. │   - Alice            │   │                     │    │
│    (bob edit)│   - Bob█             │   │  Attendees          │    │
│              │   - Carol            │   │  ─────────          │    │
│  ▼ project-  │                      │   │  • Alice            │    │
│    beta/     │   ## Action Items    │   │  • Bob              │    │
│    spec.md   │                      │   │  • Carol            │    │
│              │   1. Review PR #42   │   │                     │    │
│              │   2. Update docs     │   │  Action Items       │    │
│              │   3. Deploy v2.1     │   │  ────────────       │    │
│              │                      │   │  1. Review PR #42   │    │
│              │                      │   │  2. Update docs     │    │
│              │                      │   │  3. Deploy v2.1     │    │
│              │                      │   │                     │    │
│              │                      │   └─────────────────────┘    │
│              │                      │                               │
├──────────────┴──────────────────────┴───────────────────────────────┤
│  STATUS: connected │ spec.md saved 2s ago │ ↑↓ navigate │ e edit   │
└─────────────────────────────────────────────────────────────────────┘
```

**Key UI elements:**

- **File tree** (left): shows document hierarchy, icons for who's editing what
- **Editor** (center): raw markdown editing with syntax highlighting
- **Preview** (right): live-rendered markdown, updates on each keystroke or on save depending on method
- **Status bar**: connection state, users online, last sync time
- **User presence**: colored cursors/names showing who is in which file

---

## 5. Comparison of Methods

| Criteria | A: SSH + FS | B: Client-Server | C: P2P CRDT |
|----------|------------|-------------------|-------------|
| Setup complexity | Very low | Medium | High |
| Server requirements | sshd only | Custom daemon | Optional relay |
| Concurrent same-file editing | No (lock-based) | Yes (OT) | Yes (CRDT) |
| Offline editing | No | No | Yes |
| Network requirements | SSH access | WebSocket | P2P / relay |
| Implementation effort | Days (glue scripts) | Weeks | Months |
| Conflict resolution | Last-write-wins or lock | OT transforms | CRDT auto-merge |
| Latency sensitivity | Low (local FS ops) | Medium | Low-medium |
| Scalability | Limited by SSH sessions | Good (hundreds) | Good (mesh) |

---

## 6. Complete Inventory of Existing CLI Tools

### File Browsing & Navigation

| Tool | Description | Install |
|------|-------------|---------|
| **yazi** | Blazing-fast terminal file manager (Rust, async I/O) | `cargo install yazi-fm` |
| **lf** | Terminal file manager inspired by ranger (Go) | `go install github.com/gokcehan/lf@latest` |
| **ranger** | Console file manager with vi keybindings (Python) | `pip install ranger-fm` |
| **broot** | Tree-view file navigator with fuzzy search | `cargo install broot` |
| **xplr** | Hackable, minimal TUI file explorer (Rust) | `cargo install xplr` |

### Markdown Viewing

| Tool | Description | Install |
|------|-------------|---------|
| **glow** | Render markdown beautifully in the terminal | `go install github.com/charmbracelet/glow@latest` |
| **mdcat** | cat for markdown, with images and links | `cargo install mdcat` |
| **bat** | cat clone with syntax highlighting (supports .md) | `cargo install bat` |
| **rich-cli** | Rich text rendering in terminal (Python) | `pip install rich-cli` |

### Terminal Editors with Markdown Support

| Tool | Description | Install |
|------|-------------|---------|
| **helix** | Post-modern modal editor, tree-sitter built-in | `cargo install helix` or package manager |
| **neovim** | Extensible vim with Lua plugins, LSP, tree-sitter | Package manager |
| **micro** | Modern, intuitive terminal editor | `curl https://getmic.ro \| bash` |
| **kakoune** | Selection-based modal editor | Package manager |

### Terminal Multiplexers

| Tool | Description | Install |
|------|-------------|---------|
| **tmux** | Terminal multiplexer, session/window/pane management | Package manager |
| **zellij** | Modern terminal multiplexer with floating panes, Rust | `cargo install zellij` |
| **screen** | Classic terminal multiplexer | Package manager |

### Shared Terminal Sessions

| Tool | Description | Install |
|------|-------------|---------|
| **tmate** | Instant terminal sharing (fork of tmux) | `brew install tmate` or package manager |
| **wemux** | Multi-user tmux wrapper (host/client/pair modes) | `brew install wemux` |
| **upterm** | Secure terminal sharing via SSH/WebSocket | `brew install upterm` |

### Filesystem Watching & Sync

| Tool | Description | Install |
|------|-------------|---------|
| **entr** | Run commands when files change | Package manager |
| **watchexec** | Execute commands on file modifications (Rust) | `cargo install watchexec-cli` |
| **inotifywait** | Linux inotify CLI (inotify-tools) | `apt install inotify-tools` |
| **fswatch** | Cross-platform file change monitor | `brew install fswatch` |
| **rsync** | File synchronization (delta transfer) | Pre-installed on most systems |
| **unison** | Bidirectional file synchronization | Package manager |
| **syncthing** | Continuous P2P file sync | Package manager |
| **mutagen** | Fast file sync for remote dev | `brew install mutagen` |

### Remote Filesystem Access

| Tool | Description | Install |
|------|-------------|---------|
| **sshfs** | Mount remote dirs locally via SFTP/SSH | `apt install sshfs` |
| **rclone** | Mount cloud/remote storage as local filesystem | `curl https://rclone.org/install.sh \| bash` |
| **mosh** | Mobile shell — persistent SSH with local echo | `apt install mosh` |

### Collaborative Editing Plugins

| Tool | Description | Install |
|------|-------------|---------|
| **crdt.el** | Emacs CRDT collaborative editing mode | GNU ELPA: `M-x package-install crdt` |
| **Tandem** | CRDT collab for Neovim/Sublime/Vim | GitHub: `typeintandem/tandem` |
| **instant.nvim** | Collaborative editing for Neovim | GitHub: `jbyuki/instant.nvim` |

---

## 7. Rust Crate Dependency Map (for building from scratch)

If building a dedicated tool, here is the full crate ecosystem:

### Core TUI

| Crate | Purpose | Version |
|-------|---------|---------|
| `ratatui` | Terminal UI framework (successor to tui-rs) | 0.29+ |
| `crossterm` | Cross-platform terminal manipulation backend | 0.28+ |
| `tui-textarea` | Editable text area widget for ratatui | 0.7+ |

### Markdown Processing

| Crate | Purpose | Version |
|-------|---------|---------|
| `pulldown-cmark` | CommonMark parser, fast and correct | 0.12+ |
| `termimad` | Render markdown to styled terminal output | 0.30+ |
| `syntect` | Syntax highlighting (for code blocks) | 5.x |
| `tree-sitter` + `tree-sitter-markdown` | Incremental markdown parsing | 0.24+ |

### Networking

| Crate | Purpose | Version |
|-------|---------|---------|
| `tokio` | Async runtime | 1.x |
| `tokio-tungstenite` | Async WebSocket client/server | 0.24+ |
| `axum` | Web framework for server daemon | 0.8+ |
| `iroh` | P2P networking with NAT traversal (n0 project) | 0.30+ |
| `quinn` | QUIC protocol implementation | 0.11+ |

### CRDT / Sync

| Crate | Purpose | Version |
|-------|---------|---------|
| `yrs` (y-crdt) | Yjs CRDT port — text, arrays, maps | 0.21+ |
| `automerge` | JSON-like CRDT document model | 0.5+ |
| `similar` | Text diffing (Myers diff, patience diff) | 2.x |

### File System

| Crate | Purpose | Version |
|-------|---------|---------|
| `notify` | Cross-platform filesystem watcher | 7.x |
| `walkdir` | Recursive directory traversal | 2.x |
| `ignore` | Respects .gitignore rules when walking dirs | 0.4+ |

### SSH (for Method A)

| Crate | Purpose | Version |
|-------|---------|---------|
| `openssh` | Control a persistent SSH master connection | 0.11+ |
| `russh` | Pure Rust SSH implementation | 0.46+ |

### Serialization & Config

| Crate | Purpose | Version |
|-------|---------|---------|
| `serde` + `serde_json` | Serialization framework | 1.x |
| `toml` | Config file parsing | 0.8+ |
| `directories` | XDG/standard config paths | 5.x |

---

## 8. Recommended Starting Point

**For immediate use (today, no code):** Method A with existing tools.

```
Server setup:
  apt install tmux inotify-tools
  mkdir -p /srv/docs && chmod 770 /srv/docs

Each user:
  ssh user@server
  tmux new -s editing
  # Pane 1: helix /srv/docs/my-file.md
  # Pane 2: ls /srv/docs/*.md | entr -r glow /srv/docs/my-file.md
  # Pane 3: yazi /srv/docs
```

**For a weekend project:** Build a minimal Rust TUI using Method A (SSH + filesystem), with:
- `ratatui` + `crossterm` for the UI
- `pulldown-cmark` + `termimad` for markdown preview
- `notify` for filesystem watching
- Three-pane layout: file tree, editor, preview
- Simple `.lock` file mechanism for conflict avoidance

**For a serious project:** Method B (client-server with WebSocket) or Method C (P2P CRDT), using `yrs` for the sync engine and `iroh` or `axum`+`tokio-tungstenite` for networking.

---

## 9. Open Questions

- **Granularity of collaboration**: per-file locking (Method A), per-document OT (Method B), or per-character CRDT (Method C)?
- **Authentication**: SSH keys (Method A) are simple. Methods B/C would need token-based auth or mTLS.
- **Persistence**: who is the source of truth? Server filesystem, or CRDT state that gets flushed to disk periodically?
- **Rendering approach**: should the preview update on every keystroke (expensive but satisfying) or on save/debounce (cheaper, less fluid)?
- **Image support**: markdown can reference images — how to handle binary assets in a sync-friendly way?
- **Mobile access**: a TUI is inherently terminal-based — should there be a companion web view for read-only access from a phone?

---

## 10. References & Inspiration

- **yazi**: https://github.com/sxyazi/yazi — async terminal file manager
- **glow**: https://github.com/charmbracelet/glow — terminal markdown renderer
- **ratatui**: https://github.com/ratatui/ratatui — Rust TUI framework
- **y-crdt (yrs)**: https://github.com/y-crdt/y-crdt — Yjs CRDT in Rust
- **automerge**: https://github.com/automerge/automerge — CRDT document engine
- **iroh**: https://github.com/n0-computer/iroh — P2P networking toolkit
- **tmate**: https://tmate.io — instant terminal sharing
- **HedgeDoc**: https://hedgedoc.org — collaborative markdown editor (web, for comparison)
- **crdt.el**: https://code.librehq.com/qhong/crdt.el — Emacs CRDT mode
- **Tandem**: https://github.com/typeintandem/tandem — collaborative editing for editors
- **instant.nvim**: https://github.com/jbyuki/instant.nvim — Neovim collaborative editing
- **Xi Editor** (archived): https://github.com/xi-editor/xi-editor — CRDT-based editor architecture (influential design, now archived)
