use std::io::{self, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};

use crate::sync::SyncClient;

struct FileEntry {
    path: String,
    depth: usize,
    is_dir: bool,
    expanded: bool,
}

pub struct Browser {
    client: SyncClient,
    entries: Vec<FileEntry>,
    cursor: usize,
    scroll: usize,
    search: Option<String>,
    search_input: String,
    in_search: bool,
}

impl Browser {
    pub fn new(client: SyncClient) -> Result<Self, String> {
        let files = client.list_remote_files()?;
        let entries = build_tree(&files);

        Ok(Self {
            client,
            entries,
            cursor: 0,
            scroll: 0,
            search: None,
            search_input: String::new(),
            in_search: false,
        })
    }

    /// Run the browser TUI. Returns Some(relative_path) when a file is selected, None on quit.
    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<Option<String>> {
        loop {
            let visible = self.visible_entries();

            terminal.draw(|frame| {
                let area = frame.area();

                let title = format!(
                    " wiremd — {}:{} ",
                    self.client.host(),
                    self.client.docs_path()
                );

                let bottom = if self.in_search {
                    format!(" /{}█ ", self.search_input)
                } else {
                    " j/k: navigate │ Enter: open │ /: search │ q: quit ".to_string()
                };

                let block = Block::default()
                    .title(title)
                    .title_bottom(bottom)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray));

                let inner = block.inner(area);
                let visible_height = inner.height as usize;

                // Auto-scroll
                if self.cursor < self.scroll {
                    self.scroll = self.cursor;
                }
                if self.cursor >= self.scroll + visible_height {
                    self.scroll = self.cursor - visible_height + 1;
                }

                let content_width = inner.width as usize;
                let mut lines: Vec<Line> = Vec::new();

                for (i, idx) in visible.iter().enumerate().skip(self.scroll).take(visible_height) {
                    let entry = &self.entries[*idx];
                    let indent = "  ".repeat(entry.depth);

                    let (icon, style) = if entry.is_dir {
                        let arrow = if entry.expanded { "▼ " } else { "▶ " };
                        (
                            format!("{}{}", indent, arrow),
                            Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
                        )
                    } else {
                        (
                            format!("{}  ", indent),
                            Style::default().fg(Color::White),
                        )
                    };

                    let name = entry.path.rsplit('/').next().unwrap_or(&entry.path);

                    if i == self.cursor {
                        let cursor_bg = Color::Rgb(40, 40, 55);
                        let text = format!("{}{}", icon, name);
                        let padded = if text.len() < content_width {
                            format!("{}{}", text, " ".repeat(content_width - text.len()))
                        } else {
                            text
                        };
                        lines.push(Line::from(Span::styled(padded, style.bg(cursor_bg))));
                    } else {
                        lines.push(Line::from(vec![
                            Span::styled(icon, Style::default().fg(Color::DarkGray)),
                            Span::styled(name.to_string(), style),
                        ]));
                    }
                }

                frame.render_widget(block, area);
                let paragraph = Paragraph::new(lines);
                frame.render_widget(paragraph, inner);
            })?;

            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if self.in_search {
                    match key.code {
                        KeyCode::Esc => {
                            self.in_search = false;
                            self.search = None;
                            self.search_input.clear();
                            self.cursor = 0;
                        }
                        KeyCode::Enter => {
                            self.in_search = false;
                            if self.search_input.is_empty() {
                                self.search = None;
                            } else {
                                self.search = Some(self.search_input.clone());
                            }
                            self.cursor = 0;
                        }
                        KeyCode::Backspace => {
                            self.search_input.pop();
                            self.search = if self.search_input.is_empty() {
                                None
                            } else {
                                Some(self.search_input.clone())
                            };
                            self.cursor = 0;
                        }
                        KeyCode::Char(c) => {
                            self.search_input.push(c);
                            self.search = Some(self.search_input.clone());
                            self.cursor = 0;
                        }
                        _ => {}
                    }
                    continue;
                }

                let visible = self.visible_entries();
                let total = visible.len();

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Char('/') => {
                        self.in_search = true;
                        self.search_input.clear();
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.cursor < total.saturating_sub(1) {
                            self.cursor += 1;
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.cursor = self.cursor.saturating_sub(1);
                    }
                    KeyCode::Enter => {
                        if let Some(&idx) = visible.get(self.cursor) {
                            let entry = &self.entries[idx];
                            if entry.is_dir {
                                // Toggle expand
                                self.entries[idx].expanded = !self.entries[idx].expanded;
                            } else {
                                // Select file
                                return Ok(Some(entry.path.clone()));
                            }
                        }
                    }
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        let visible_height = terminal.size()?.height.saturating_sub(2) as usize;
                        self.cursor = (self.cursor + visible_height).min(total.saturating_sub(1));
                    }
                    KeyCode::PageUp => {
                        let visible_height = terminal.size()?.height.saturating_sub(2) as usize;
                        self.cursor = self.cursor.saturating_sub(visible_height);
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        self.cursor = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        self.cursor = total.saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Fetch a file from the remote server
    pub fn fetch_file(&self, relative_path: &str) -> Result<String, String> {
        self.client.read_remote_file(relative_path)
    }

    /// Get visible entries (filtered by search, respecting expand state)
    fn visible_entries(&self) -> Vec<usize> {
        let mut result = Vec::new();
        let mut collapsed_depth: Option<usize> = None;

        for (i, entry) in self.entries.iter().enumerate() {
            // Skip children of collapsed dirs
            if let Some(cd) = collapsed_depth {
                if entry.depth > cd {
                    continue;
                } else {
                    collapsed_depth = None;
                }
            }

            // If dir is collapsed, skip its children
            if entry.is_dir && !entry.expanded {
                collapsed_depth = Some(entry.depth);
            }

            // Apply search filter
            if let Some(ref query) = self.search {
                let name = entry.path.to_lowercase();
                let q = query.to_lowercase();
                if !name.contains(&q) && !entry.is_dir {
                    continue;
                }
                // Always show dirs (they might contain matching files)
                // but only if they have visible children — simplified: show all dirs
            }

            result.push(i);
        }

        result
    }
}

/// Build a flat tree from sorted file paths.
/// Inserts directory entries and computes depths.
fn build_tree(files: &[String]) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut seen_dirs = std::collections::HashSet::new();

    for file in files {
        let parts: Vec<&str> = file.split('/').collect();

        // Insert directory entries for each path component
        for i in 0..parts.len() - 1 {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(FileEntry {
                    path: dir_path,
                    depth: i,
                    is_dir: true,
                    expanded: true,
                });
            }
        }

        // Insert the file entry
        entries.push(FileEntry {
            path: file.clone(),
            depth: parts.len() - 1,
            is_dir: false,
            expanded: false,
        });
    }

    entries
}
