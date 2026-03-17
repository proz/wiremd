# wiremd

A terminal-native collaborative markdown editor built in Rust. Browse, edit, and preview markdown documents with real-time CRDT-based sync over SSH.

## Features

- **Markdown viewer** with styled rendering (headings, code blocks, tables, lists, links, etc.)
- **Word wrapping** at configurable width
- **Code blocks** with distinct background and indentation
- **Tables** with aligned columns and box-drawing borders
- **Scrollable** with vim-style keybindings
- **Collaborative editing** via yrs CRDT sync over SSH (planned)

## Install

```bash
git clone https://github.com/proz/wiremd.git
cd wiremd
cargo build --release
```

The binary will be at `target/release/wiremd`.

## Usage

```bash
wiremd <file.md>
```

### Keybindings

| Key | Action |
|-----|--------|
| `j` / `↓` | Scroll down |
| `k` / `↑` | Scroll up |
| `Space` / `PageDown` | Page down |
| `PageUp` | Page up |
| `g` / `Home` | Go to top |
| `G` / `End` | Go to bottom |
| `q` / `Esc` | Quit |

## Dependencies

- [ratatui](https://github.com/ratatui/ratatui) — TUI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) — terminal backend
- [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark) — markdown parser

## Roadmap

- [ ] Three-pane layout (file tree, editor, preview)
- [ ] Markdown editing with tui-textarea
- [ ] yrs CRDT integration for collaborative editing
- [ ] SSH-based sync (push/pull deltas to shared server)
- [ ] User presence awareness
- [ ] Configurable themes and styles

## License

MIT
