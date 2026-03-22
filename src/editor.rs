use std::io::{self, Stdout};
use std::process::Child;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use similar::{ChangeTag, TextDiff};
use tui_textarea::TextArea;
use yrs::{Doc, GetString, ReadTxn, Text, TextRef, Transact, updates::decoder::Decode};

use crate::sync::SyncClient;

const MAX_WIDTH: usize = 80;

enum Mode {
    View,
    Edit,
}

/// Maps display lines back to source positions
struct DisplayMap {
    lines: Vec<Line<'static>>,
    /// For each display line: (source_line_index, col_offset_within_source)
    map: Vec<(usize, usize)>,
}

impl DisplayMap {
    /// Find display line index for a given source (row, col)
    fn source_to_display(&self, src_row: usize, src_col: usize) -> (usize, usize) {
        let mut best_display_row = 0;
        for (i, &(sline, scol)) in self.map.iter().enumerate() {
            if sline == src_row {
                // Check if this display line contains our column
                let next_col = self
                    .map
                    .get(i + 1)
                    .filter(|&&(nl, _)| nl == src_row)
                    .map(|&(_, nc)| nc)
                    .unwrap_or(usize::MAX);
                if src_col >= scol && src_col < next_col {
                    return (i, src_col - scol);
                }
                best_display_row = i;
            } else if sline > src_row {
                break;
            }
        }
        // Fallback: last display line for this source line
        (best_display_row, src_col.saturating_sub(self.map.get(best_display_row).map(|m| m.1).unwrap_or(0)))
    }

    /// Find source line for a given display line index
    fn display_to_source(&self, display_row: usize) -> usize {
        self.map.get(display_row).map(|m| m.0).unwrap_or(0)
    }

    fn len(&self) -> usize {
        self.lines.len()
    }
}

pub struct Editor {
    path: String,
    relative_path: String,
    textarea: TextArea<'static>,
    doc: Doc,
    text: TextRef,
    mode: Mode,
    scroll: usize,
    view_cursor: usize,
    modified: bool,
    sync_status: &'static str,
    sync_client: Option<SyncClient>,
    last_synced_content: String,
    pending_updates: Vec<Vec<u8>>,
    updates: std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    _sub: yrs::Subscription,
    watcher_rx: Option<mpsc::Receiver<Vec<u8>>>,
    _watcher_child: Option<Child>,
    user_name: String,
    online_users: Vec<String>,
    last_save: Instant,
}

impl Editor {
    pub fn new(
        path: String,
        content: String,
        sync_client: Option<SyncClient>,
        relative_path: String,
        user_name: String,
    ) -> Self {
        let doc = Doc::new();
        let text = doc.get_or_insert_text("content");

        let mut sync_status: &'static str = if sync_client.is_some() {
            "connected"
        } else {
            "offline"
        };

        // Try to pull existing yrs state from server (so all clients share the same base)
        let mut initial_content = content.clone();
        if let Some(ref client) = sync_client {
            let _ = client.ensure_remote_dirs(&relative_path);

            if let Ok(Some(remote_state)) = client.pull_state(&relative_path) {
                if let Ok(update) = yrs::Update::decode_v1(&remote_state) {
                    {
                        let mut txn = doc.transact_mut();
                        let _ = txn.apply_update(update);
                    } // drop write txn before reading
                    let txn = doc.transact();
                    let remote_content = text.get_string(&txn);
                    if !remote_content.is_empty() {
                        initial_content = remote_content;
                        sync_status = "synced";
                    }
                }
            }
        }

        // Reflow for editing
        let reflowed = reflow(&initial_content, MAX_WIDTH);

        // Create textarea first — its content format is the canonical form
        let mut textarea = TextArea::from(
            reflowed.lines().map(|l| l.to_string()).collect::<Vec<_>>(),
        );
        textarea.set_cursor_line_style(Style::default());
        textarea.set_cursor_style(Style::default());

        // Get the exact string that textarea_content() will produce
        // This is the single source of truth for content format
        let canonical = textarea_content(&textarea);

        // Initialize yrs doc to match the canonical content exactly
        {
            let txn = doc.transact();
            let current_yrs = text.get_string(&txn);
            drop(txn);

            if current_yrs.is_empty() {
                let mut txn = doc.transact_mut();
                text.insert(&mut txn, 0, &canonical);
            } else if current_yrs != canonical {
                sync_to_yrs(&text, &doc, &current_yrs, &canonical);
            }
        }

        let synced_content = canonical;

        let updates = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
        let updates_clone = updates.clone();
        let _sub = doc.observe_update_v1(move |_txn, event| {
            updates_clone.lock().unwrap().push(event.update.clone());
        }).unwrap();

        // Start file watcher and set presence
        let (watcher_rx, watcher_child) = if let Some(ref client) = sync_client {
            let _ = client.set_presence(&relative_path, &user_name);
            match client.watch_remote(&relative_path) {
                Ok((rx, child)) => (Some(rx), Some(child)),
                Err(_) => (None, None),
            }
        } else {
            (None, None)
        };

        let online_users = if let Some(ref client) = sync_client {
            client.list_presence(&relative_path).unwrap_or_default()
        } else {
            Vec::new()
        };

        Self {
            path,
            relative_path,
            textarea,
            doc,
            text,
            mode: Mode::View,
            scroll: 0,
            view_cursor: 0,
            modified: false,
            sync_status,
            sync_client,
            last_synced_content: synced_content,
            pending_updates: Vec::new(),
            updates,
            _sub,
            watcher_rx,
            _watcher_child: watcher_child,
            user_name,
            online_users,
            last_save: Instant::now(),
        }
    }

    pub fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        let autosave_interval = Duration::from_secs(3);

        loop {
            // Check for remote changes (pre-pulled by background thread)
            if let Some(ref rx) = self.watcher_rx {
                let mut latest_state: Option<Vec<u8>> = None;
                while let Ok(state) = rx.try_recv() {
                    latest_state = Some(state);
                }
                if let Some(state) = latest_state {
                    self.apply_remote_state(&state);
                }
            }

            // Auto-save when modified and enough time has passed
            if self.modified && self.last_save.elapsed() >= autosave_interval {
                self.do_autosave();
            }

            self.draw(terminal)?;

            if !event::poll(Duration::from_millis(200))? {
                continue;
            }

            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if self.handle_key(key, terminal)? {
                    break;
                }
            }
        }

        // Save on exit if modified
        if self.modified {
            self.do_autosave();
        }

        // Clean up presence on exit
        if let Some(ref client) = self.sync_client {
            let _ = client.clear_presence(&self.relative_path, &self.user_name);
        }

        // Kill watcher process
        if let Some(ref mut child) = self._watcher_child {
            let _ = child.kill();
        }

        Ok(())
    }

    /// Auto-save: sync to yrs, write locally (fast), push to server in background thread
    fn do_autosave(&mut self) {
        // 1. Sync textarea to yrs (in-memory, fast)
        let local_content = textarea_content(&self.textarea);
        sync_to_yrs(&self.text, &self.doc, &self.last_synced_content, &local_content);
        self.last_synced_content = local_content.clone();

        // Drain observer updates
        {
            let mut u = self.updates.lock().unwrap();
            self.pending_updates.extend(u.drain(..));
        }

        // 2. Write locally (fast)
        let _ = std::fs::write(&self.path, &local_content);

        // 3. Push to server in background (non-blocking)
        if let Some(ref client) = self.sync_client {
            let state = {
                let txn = self.doc.transact();
                txn.encode_state_as_update_v1(&yrs::StateVector::default())
            };

            // Spawn background thread for SSH push
            let host = client.host().to_string();
            let ssh_user = client.ssh_user().to_string();
            let port = client.port();
            let docs_path = client.docs_path().to_string();
            let relative_path = self.relative_path.clone();
            let content = local_content.clone();

            std::thread::spawn(move || {
                // Push yrs state
                let yrs_remote = format!(
                    "{}@{}:{}/.wiremd/{}.yrs",
                    ssh_user, host, docs_path, relative_path
                );
                let tmp_state = std::env::temp_dir().join("wiremd_autosave_state");
                if std::fs::write(&tmp_state, &state).is_ok() {
                    let _ = std::process::Command::new("scp")
                        .arg("-P").arg(port.to_string())
                        .arg("-o").arg("BatchMode=yes")
                        .arg("-o").arg("ConnectTimeout=5")
                        .arg(tmp_state.to_str().unwrap())
                        .arg(&yrs_remote)
                        .output();
                    let _ = std::fs::remove_file(&tmp_state);
                }

                // Push markdown file
                let file_remote = format!(
                    "{}@{}:{}/{}",
                    ssh_user, host, docs_path, relative_path
                );
                let tmp_file = std::env::temp_dir().join("wiremd_autosave_file");
                if std::fs::write(&tmp_file, &content).is_ok() {
                    let _ = std::process::Command::new("scp")
                        .arg("-P").arg(port.to_string())
                        .arg("-o").arg("BatchMode=yes")
                        .arg("-o").arg("ConnectTimeout=5")
                        .arg(tmp_file.to_str().unwrap())
                        .arg(&file_remote)
                        .output();
                    let _ = std::fs::remove_file(&tmp_file);
                }
            });

            self.sync_status = "saving...";
        } else {
            self.sync_status = "saved";
        }

        self.pending_updates.clear();
        self.modified = false;
        self.last_save = Instant::now();
    }

    /// Apply a pre-pulled remote state (no I/O, fast)
    fn apply_remote_state(&mut self, remote_state: &[u8]) {
        if let Ok(update) = yrs::Update::decode_v1(remote_state) {
            {
                let mut txn = self.doc.transact_mut();
                let _ = txn.apply_update(update);
            }

            let merged = {
                let txn = self.doc.transact();
                self.text.get_string(&txn)
            };

            let current = textarea_content(&self.textarea);
            if merged != current {
                let cursor_pos = self.textarea.cursor();
                self.reload_textarea(&merged);
                self.textarea.move_cursor(tui_textarea::CursorMove::Jump(
                    cursor_pos.0 as u16, cursor_pos.1 as u16,
                ));
                self.sync_status = "live";
            }
        }
    }

    /// Handle a key event. Returns true if the editor should exit.
    fn handle_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    ) -> io::Result<bool> {
        let display = highlight_and_wrap(self.textarea.lines(), MAX_WIDTH);
        let total_display_lines = display.len();
        let visible_height = terminal.size()?.height.saturating_sub(2) as usize;

        match self.mode {
            Mode::View => {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
                    KeyCode::Char('e') | KeyCode::Enter => {
                        let src_line = display.display_to_source(self.view_cursor);
                        self.textarea.move_cursor(
                            tui_textarea::CursorMove::Jump(src_line as u16, 0),
                        );
                        self.mode = Mode::Edit;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.view_cursor < total_display_lines.saturating_sub(1) {
                            self.view_cursor += 1;
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.view_cursor = self.view_cursor.saturating_sub(1);
                    }
                    KeyCode::PageDown | KeyCode::Char(' ') => {
                        self.view_cursor = (self.view_cursor + visible_height)
                            .min(total_display_lines.saturating_sub(1));
                    }
                    KeyCode::PageUp => {
                        self.view_cursor = self.view_cursor.saturating_sub(visible_height);
                    }
                    KeyCode::Home | KeyCode::Char('g') => {
                        self.view_cursor = 0;
                    }
                    KeyCode::End | KeyCode::Char('G') => {
                        self.view_cursor = total_display_lines.saturating_sub(1);
                    }
                    _ => {}
                }
            }
            Mode::Edit => {
                if key.code == KeyCode::Esc {
                    let (row, col) = self.textarea.cursor();
                    let (dr, _) = display.source_to_display(row, col);
                    self.view_cursor = dr;
                    self.mode = Mode::View;
                    return Ok(false);
                }

                let event = Event::Key(key);
                if self.textarea.input(event) {
                    self.modified = true;
                }
            }
        }

        Ok(false)
    }

    fn draw(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
        let path = self.path.clone();
        let sync_status = self.sync_status;
        let modified = self.modified;
        let pending_count = self.pending_updates.len();

        // Build online users string
        let users_info = if !self.online_users.is_empty() {
            format!(" [{}]", self.online_users.join(", "))
        } else {
            String::new()
        };

        terminal.draw(|frame| {
            let area = frame.area();
            let lines = self.textarea.lines();

                let updates_count = pending_count;
                let sync_info = if updates_count > 0 {
                    format!(" [{}|{} pending]", sync_status, updates_count)
                } else {
                    format!(" [{}]", sync_status)
                };

                let title = match self.mode {
                    Mode::View => {
                        if modified {
                            format!(" {} [modified]{}{} ", path, sync_info, users_info)
                        } else {
                            format!(" {}{}{} ", path, sync_info, users_info)
                        }
                    }
                    Mode::Edit => {
                        if modified {
                            format!(" {} [editing] [modified]{}{} ", path, sync_info, users_info)
                        } else {
                            format!(" {} [editing]{}{} ", path, sync_info, users_info)
                        }
                    }
                };

                let bottom = match self.mode {
                    Mode::View => " e: edit │ q: quit ",
                    Mode::Edit => " Esc: view │ auto-saving ",
                };

                let block = Block::default()
                    .title(title)
                    .title_bottom(bottom)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray));

                let inner = block.inner(area);
                let visible_height = inner.height as usize;

                let display = highlight_and_wrap(lines, MAX_WIDTH);

                let (display_cursor_line, display_cursor_col) = match self.mode {
                    Mode::View => (self.view_cursor, None),
                    Mode::Edit => {
                        let (row, col) = self.textarea.cursor();
                        let (dr, dc) = display.source_to_display(row, col);
                        (dr, Some(dc))
                    }
                };

                if display_cursor_line < self.scroll {
                    self.scroll = display_cursor_line;
                }
                if display_cursor_line >= self.scroll + visible_height {
                    self.scroll = display_cursor_line - visible_height + 1;
                }

                let mut display_lines: Vec<Line> = Vec::new();
                let content_width = inner.width as usize;

                for (i, line) in display.lines.iter().enumerate().skip(self.scroll).take(visible_height) {
                    if i == display_cursor_line {
                        let cursor_bg = Color::Rgb(40, 40, 55);
                        let mut spans: Vec<Span<'static>> = line
                            .spans
                            .iter()
                            .map(|span| {
                                Span::styled(
                                    span.content.to_string(),
                                    span.style.bg(cursor_bg),
                                )
                            })
                            .collect();

                        let text_len: usize = spans.iter().map(|s| s.content.len()).sum();
                        if text_len < content_width {
                            spans.push(Span::styled(
                                " ".repeat(content_width - text_len),
                                Style::default().bg(cursor_bg),
                            ));
                        }

                        display_lines.push(Line::from(spans));
                    } else {
                        display_lines.push(line.clone());
                    }
                }

                frame.render_widget(block, area);
                let paragraph = Paragraph::new(display_lines);
                frame.render_widget(paragraph, inner);

                if let Some(col) = display_cursor_col {
                    let screen_row = display_cursor_line.saturating_sub(self.scroll) as u16;
                    let screen_col = col as u16;
                    if screen_row < inner.height && screen_col < inner.width {
                        let cx = inner.x + screen_col;
                        let cy = inner.y + screen_row;
                        if let Some(cell) = frame.buffer_mut().cell_mut((cx, cy)) {
                            cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
                        }
                    }
                }
        })?;
        Ok(())
    }

    /// Full save + sync flow using yrs state snapshots (not individual updates).
    /// 1. Sync local edits to yrs doc
    /// 2. Pull remote yrs state from server
    /// 3. Merge remote state into local doc (CRDT auto-merge, idempotent)
    /// 4. Get merged text from yrs doc
    /// 5. Write merged text locally
    /// 6. Push local yrs state to server (full snapshot)
    /// 7. Push merged markdown file to server
    /// Reload the textarea with new content, update last_synced_content to match
    fn reload_textarea(&mut self, content: &str) {
        self.textarea = TextArea::from(
            content.lines().map(|l| l.to_string()).collect::<Vec<_>>(),
        );
        self.textarea.set_cursor_line_style(Style::default());
        self.textarea.set_cursor_style(Style::default());
        // Keep last_synced_content in sync with textarea_content()
        self.last_synced_content = textarea_content(&self.textarea);
    }

}

/// Reflow: wrap long paragraph lines at max_width for editing.
/// Block-level elements (headings, lists, code, tables, blank lines) are left as-is.
fn reflow(input: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut in_code_block = false;

    for line in input.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            result.push_str(line);
            result.push('\n');
            continue;
        }

        // Don't wrap: code blocks, block elements, short lines
        if in_code_block || is_block_element(line) || line.len() <= max_width {
            result.push_str(line);
            result.push('\n');
            continue;
        }

        // Wrap long paragraph line at word boundaries
        let mut pos = 0;
        let bytes = line.as_bytes();
        while pos < bytes.len() {
            if bytes.len() - pos <= max_width {
                result.push_str(&line[pos..]);
                result.push('\n');
                break;
            }

            // Find last space before max_width
            let end = pos + max_width;
            let mut break_at = end;
            for i in (pos..end).rev() {
                if bytes[i] == b' ' {
                    break_at = i;
                    break;
                }
            }

            // No space found -- force break at max_width
            if break_at == end && break_at < bytes.len() {
                result.push_str(&line[pos..break_at]);
                result.push('\n');
                pos = break_at;
            } else {
                result.push_str(&line[pos..break_at]);
                result.push('\n');
                pos = break_at + 1; // skip the space
            }
        }
    }

    result
}

/// Check if a line is a markdown block element (should not be joined with adjacent lines).
fn is_block_element(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    // Headings
    if trimmed.starts_with('#') && trimmed.contains("# ") {
        return true;
    }
    // Lists
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    // Ordered lists
    if let Some(dot_pos) = trimmed.find(". ") {
        if trimmed[..dot_pos].chars().all(|c| c.is_ascii_digit()) && dot_pos > 0 {
            return true;
        }
    }
    // Block quotes
    if trimmed.starts_with('>') {
        return true;
    }
    // Tables
    if trimmed.starts_with('|') {
        return true;
    }
    // Horizontal rules
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return true;
    }
    // Code fences
    if trimmed.starts_with("```") {
        return true;
    }
    false
}

/// Diff old vs new content and apply the changes to the yrs Text type.
fn sync_to_yrs(text: &TextRef, doc: &Doc, old: &str, new: &str) {
    if old == new {
        return;
    }

    // Diff line by line first, then char-level within changed lines.
    // This handles newline insertions/deletions correctly.
    let line_diff = TextDiff::from_lines(old, new);
    let mut txn = doc.transact_mut();
    let mut pos: u32 = 0;

    for change in line_diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                pos += change.value().len() as u32;
            }
            ChangeTag::Delete => {
                let len = change.value().len() as u32;
                text.remove_range(&mut txn, pos, len);
            }
            ChangeTag::Insert => {
                let value = change.value();
                text.insert(&mut txn, pos, value);
                pos += value.len() as u32;
            }
        }
    }
}

fn textarea_content(textarea: &TextArea) -> String {
    let lines = textarea.lines();
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Highlight markdown lines and soft-wrap at max_width.
fn highlight_and_wrap(lines: &[String], max_width: usize) -> DisplayMap {
    let mut display_lines: Vec<Line<'static>> = Vec::new();
    let mut map: Vec<(usize, usize)> = Vec::new();
    let mut in_code_block = false;
    let code_bg = Color::Rgb(40, 40, 40);

    for (src_idx, line) in lines.iter().enumerate() {
        // Code fence toggles
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            let padded = pad_to_width(line, max_width);
            display_lines.push(Line::from(Span::styled(
                padded,
                Style::default().fg(Color::DarkGray).bg(code_bg),
            )));
            map.push((src_idx, 0));
            continue;
        }

        // Inside code block -- no wrapping, pad to width
        if in_code_block {
            let padded = pad_to_width(line, max_width);
            display_lines.push(Line::from(Span::styled(
                padded,
                Style::default().fg(Color::Gray).bg(code_bg),
            )));
            map.push((src_idx, 0));
            continue;
        }

        // Highlight the line
        let spans = highlight_source_line(line);

        // If it fits, no wrapping needed
        let total_len: usize = spans.iter().map(|s| s.content.len()).sum();
        if total_len <= max_width {
            display_lines.push(Line::from(spans));
            map.push((src_idx, 0));
            continue;
        }

        // Soft-wrap the spans
        let wrapped = wrap_spans(&spans, max_width);
        let mut col_offset = 0usize;
        for wrapped_line in wrapped {
            let line_len: usize = wrapped_line.iter().map(|s| s.content.len()).sum();
            display_lines.push(Line::from(wrapped_line));
            map.push((src_idx, col_offset));
            col_offset += line_len;
        }
    }

    DisplayMap {
        lines: display_lines,
        map,
    }
}

/// Highlight a single source line based on markdown syntax.
fn highlight_source_line(line: &str) -> Vec<Span<'static>> {
    // Headings
    if let Some(level) = heading_level(line) {
        let color = match level {
            1 => Color::Magenta,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Cyan,
            _ => Color::Blue,
        };
        return highlight_inline(
            line,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        );
    }

    // Horizontal rules
    let trimmed = line.trim();
    if (trimmed == "---" || trimmed == "***" || trimmed == "___") && trimmed.len() >= 3 {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(Color::DarkGray),
        )];
    }

    // Block quotes
    if line.starts_with('>') {
        return highlight_inline(line, Style::default().fg(Color::Gray));
    }

    // Table lines
    if line.starts_with('|') {
        if line.contains("---") || line.contains("===") {
            return vec![Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            )];
        } else {
            return highlight_inline(line, Style::default().fg(Color::White));
        }
    }

    // List items
    if is_list_item(line) {
        return highlight_list_item(line);
    }

    // Regular text
    highlight_inline(line, Style::default())
}

/// Word-wrap a list of styled spans at max_width.
fn wrap_spans(spans: &[Span<'static>], max_width: usize) -> Vec<Vec<Span<'static>>> {
    let mut result: Vec<Vec<Span<'static>>> = Vec::new();
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut current_len: usize = 0;

    for span in spans {
        let style = span.style;
        let text = span.content.to_string();

        for word in WordSplitter::new(&text) {
            let word_len = word.len();

            if current_len + word_len > max_width && current_len > 0 {
                result.push(std::mem::take(&mut current_line));
                current_len = 0;

                let trimmed = word.trim_start();
                if !trimmed.is_empty() {
                    current_len = trimmed.len();
                    current_line.push(Span::styled(trimmed.to_string(), style));
                }
            } else {
                current_len += word_len;
                current_line.push(Span::styled(word.to_string(), style));
            }
        }
    }

    if !current_line.is_empty() {
        result.push(current_line);
    }

    if result.is_empty() {
        result.push(vec![Span::raw(String::new())]);
    }

    result
}

fn heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes > 0 && hashes <= 6 && trimmed.as_bytes().get(hashes) == Some(&b' ') {
        Some(hashes)
    } else {
        None
    }
}

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return true;
    }
    if let Some(dot_pos) = trimmed.find(". ") {
        return trimmed[..dot_pos].chars().all(|c| c.is_ascii_digit()) && dot_pos > 0;
    }
    false
}

fn highlight_list_item(line: &str) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    let indent_len = line.len() - trimmed.len();
    let indent = &line[..indent_len];

    let (bullet, rest) = if trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
    {
        (&trimmed[..2], &trimmed[2..])
    } else if let Some(dot_pos) = trimmed.find(". ") {
        (&trimmed[..dot_pos + 2], &trimmed[dot_pos + 2..])
    } else {
        return highlight_inline(line, Style::default());
    };

    let mut spans = vec![
        Span::raw(indent.to_string()),
        Span::styled(bullet.to_string(), Style::default().fg(Color::Cyan)),
    ];
    spans.extend(highlight_inline(rest, Style::default()));
    spans
}

/// Parse inline markdown: **bold**, *italic*, ~~strikethrough~~, `code`, [links](url)
fn highlight_inline(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;
    let mut plain_start = 0;

    while i < len {
        // Inline code: `...`
        if chars[i] == '`' {
            if i > plain_start {
                spans.push(Span::styled(
                    chars[plain_start..i].iter().collect::<String>(),
                    base_style,
                ));
            }
            if let Some(end) = find_closing(&chars, i + 1, '`') {
                let code_text: String = chars[i..=end].iter().collect();
                spans.push(Span::styled(
                    code_text,
                    Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 40, 40)),
                ));
                i = end + 1;
                plain_start = i;
                continue;
            }
        }

        // Bold: **...**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if i > plain_start {
                spans.push(Span::styled(
                    chars[plain_start..i].iter().collect::<String>(),
                    base_style,
                ));
            }
            if let Some(end) = find_closing_double(&chars, i + 2, '*') {
                let bold_text: String = chars[i..=end + 1].iter().collect();
                spans.push(Span::styled(
                    bold_text,
                    base_style.add_modifier(Modifier::BOLD),
                ));
                i = end + 2;
                plain_start = i;
                continue;
            }
        }

        // Italic: *...*
        if chars[i] == '*' && (i + 1 < len && chars[i + 1] != '*') {
            if i > plain_start {
                spans.push(Span::styled(
                    chars[plain_start..i].iter().collect::<String>(),
                    base_style,
                ));
            }
            if let Some(end) = find_closing(&chars, i + 1, '*') {
                let italic_text: String = chars[i..=end].iter().collect();
                spans.push(Span::styled(
                    italic_text,
                    base_style.add_modifier(Modifier::ITALIC),
                ));
                i = end + 1;
                plain_start = i;
                continue;
            }
        }

        // Strikethrough: ~~...~~
        if i + 1 < len && chars[i] == '~' && chars[i + 1] == '~' {
            if i > plain_start {
                spans.push(Span::styled(
                    chars[plain_start..i].iter().collect::<String>(),
                    base_style,
                ));
            }
            if let Some(end) = find_closing_double(&chars, i + 2, '~') {
                let strike_text: String = chars[i..=end + 1].iter().collect();
                spans.push(Span::styled(
                    strike_text,
                    base_style.add_modifier(Modifier::CROSSED_OUT),
                ));
                i = end + 2;
                plain_start = i;
                continue;
            }
        }

        // Links: [text](url)
        if chars[i] == '[' {
            if i > plain_start {
                spans.push(Span::styled(
                    chars[plain_start..i].iter().collect::<String>(),
                    base_style,
                ));
            }
            if let Some((bracket_end, paren_end)) = find_link(&chars, i) {
                let link_text: String = chars[i..=bracket_end].iter().collect();
                let url_text: String = chars[bracket_end + 1..=paren_end].iter().collect();
                spans.push(Span::styled(
                    link_text,
                    base_style.fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
                ));
                spans.push(Span::styled(
                    url_text,
                    Style::default().fg(Color::DarkGray),
                ));
                i = paren_end + 1;
                plain_start = i;
                continue;
            }
        }

        i += 1;
    }

    if plain_start < len {
        spans.push(Span::styled(
            chars[plain_start..].iter().collect::<String>(),
            base_style,
        ));
    }

    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base_style));
    }

    spans
}

fn find_closing(chars: &[char], start: usize, delim: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == delim {
            return Some(i);
        }
    }
    None
}

fn find_closing_double(chars: &[char], start: usize, delim: char) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == delim && chars[i + 1] == delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_link(chars: &[char], start: usize) -> Option<(usize, usize)> {
    let mut i = start + 1;
    while i < chars.len() && chars[i] != ']' {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    let bracket_end = i;
    if bracket_end + 1 >= chars.len() || chars[bracket_end + 1] != '(' {
        return None;
    }
    i = bracket_end + 2;
    while i < chars.len() && chars[i] != ')' {
        i += 1;
    }
    if i >= chars.len() {
        return None;
    }
    Some((bracket_end, i))
}

fn pad_to_width(text: &str, width: usize) -> String {
    if text.len() >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - text.len()))
    }
}

struct WordSplitter<'a> {
    remaining: &'a str,
}

impl<'a> WordSplitter<'a> {
    fn new(text: &'a str) -> Self {
        Self { remaining: text }
    }
}

impl<'a> Iterator for WordSplitter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }

        let bytes = self.remaining.as_bytes();
        let mut i = 0;

        while i < bytes.len() && bytes[i] != b' ' {
            i += 1;
        }
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }

        if i == 0 {
            let result = self.remaining;
            self.remaining = "";
            Some(result)
        } else {
            let (chunk, rest) = self.remaining.split_at(i);
            self.remaining = rest;
            Some(chunk)
        }
    }
}
