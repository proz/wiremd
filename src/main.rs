use std::env;
use std::fs;
use std::io::{self, stdout};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use pulldown_cmark::{Event as MdEvent, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::{
    layout::{Constraint, Layout},
    prelude::CrosstermBackend,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Terminal,
};
use tui_textarea::{CursorMove, TextArea};

const MAX_WIDTH: usize = 80;
const CODE_MARGIN: usize = 4;

enum Mode {
    View,
    Edit,
}

struct RenderResult {
    lines: Vec<Line<'static>>,
    /// Maps each rendered line index to a source line number (0-based)
    source_map: Vec<usize>,
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: wiremd <file.md>");
        std::process::exit(1);
    }

    let path = &args[1];
    let content = fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("Error reading {}: {}", path, e);
        std::process::exit(1);
    });

    let mut mode = Mode::View;
    let mut render = render_markdown(&content, MAX_WIDTH);
    let mut scroll: u16 = 0;
    let mut cursor_line: u16 = 0;
    let mut modified = false;

    let mut textarea = TextArea::from(content.lines());
    textarea.set_block(
        Block::default()
            .title(format!(" {} [EDITING] ", path))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow)),
    );
    textarea.set_cursor_line_style(Style::default().bg(Color::Rgb(30, 30, 30)));
    textarea.set_line_number_style(Style::default().fg(Color::DarkGray));

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    loop {
        let total_lines = render.lines.len() as u16;

        terminal.draw(|frame| {
            let area = frame.area();

            match mode {
                Mode::View => {
                    let chunks = Layout::horizontal([
                        Constraint::Min(0),
                        Constraint::Length(1),
                    ])
                    .split(area);

                    let visible_height = chunks[0].height.saturating_sub(2);
                    let max_scroll = total_lines.saturating_sub(visible_height);

                    let title = if modified {
                        format!(" {} [modified] ", path)
                    } else {
                        format!(" {} ", path)
                    };

                    // Highlight the cursor line
                    let mut display_lines = render.lines.clone();
                    let content_width = chunks[0].width.saturating_sub(2) as usize; // inside borders
                    if (cursor_line as usize) < display_lines.len() {
                        let idx = cursor_line as usize;
                        let line = &display_lines[idx];
                        let cursor_bg = Color::Rgb(40, 40, 55);

                        let mut highlighted: Vec<Span<'static>> = line
                            .spans
                            .iter()
                            .map(|span| {
                                Span::styled(
                                    span.content.to_string(),
                                    span.style.bg(cursor_bg),
                                )
                            })
                            .collect();

                        // Pad to full width so the highlight spans the entire line
                        let text_len: usize = highlighted.iter().map(|s| s.content.len()).sum();
                        if text_len < content_width {
                            highlighted.push(Span::styled(
                                " ".repeat(content_width - text_len),
                                Style::default().bg(cursor_bg),
                            ));
                        }

                        display_lines[idx] = Line::from(highlighted);
                    }

                    let block = Block::default()
                        .title(title)
                        .title_bottom(" e: edit │ q: quit │ s: save ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::DarkGray));

                    let paragraph = Paragraph::new(display_lines)
                        .block(block)
                        .scroll((scroll, 0));

                    frame.render_widget(paragraph, chunks[0]);

                    let mut scrollbar_state =
                        ScrollbarState::new(max_scroll as usize).position(scroll as usize);
                    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
                    frame.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);
                }
                Mode::Edit => {
                    frame.render_widget(&textarea, area);
                }
            }
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match mode {
                Mode::View => {
                    let visible_height = terminal.size()?.height.saturating_sub(2);
                    let max_scroll = total_lines.saturating_sub(visible_height);

                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('e') | KeyCode::Enter => {
                            // Map cursor_line to source line via source_map
                            let source_line = render
                                .source_map
                                .get(cursor_line as usize)
                                .copied()
                                .unwrap_or(0);

                            // Move textarea cursor to the corresponding source line
                            textarea.move_cursor(CursorMove::Jump(source_line as u16, 0));

                            // Scroll textarea so the cursor appears at the same
                            // screen row it was on in view mode.
                            // Jump to the end first to force the viewport far down,
                            // then to the top-of-screen line to anchor the viewport,
                            // then to the actual cursor line.
                            let screen_offset = cursor_line.saturating_sub(scroll) as usize;
                            let top_source_line = source_line.saturating_sub(screen_offset);
                            textarea.move_cursor(CursorMove::Bottom);
                            textarea.move_cursor(CursorMove::Jump(top_source_line as u16, 0));
                            textarea.move_cursor(CursorMove::Jump(source_line as u16, 0));

                            mode = Mode::Edit;
                        }
                        KeyCode::Char('s') => {
                            if modified {
                                let text = textarea_content(&textarea);
                                fs::write(path, &text)?;
                                modified = false;
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if cursor_line < total_lines.saturating_sub(1) {
                                cursor_line += 1;
                                // Auto-scroll to keep cursor visible
                                if cursor_line >= scroll + visible_height {
                                    scroll = (cursor_line - visible_height + 1).min(max_scroll);
                                }
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            cursor_line = cursor_line.saturating_sub(1);
                            if cursor_line < scroll {
                                scroll = cursor_line;
                            }
                        }
                        KeyCode::PageDown | KeyCode::Char(' ') => {
                            cursor_line = (cursor_line + visible_height)
                                .min(total_lines.saturating_sub(1));
                            scroll = scroll.saturating_add(visible_height).min(max_scroll);
                        }
                        KeyCode::PageUp => {
                            cursor_line = cursor_line.saturating_sub(visible_height);
                            scroll = scroll.saturating_sub(visible_height);
                        }
                        KeyCode::Home | KeyCode::Char('g') => {
                            cursor_line = 0;
                            scroll = 0;
                        }
                        KeyCode::End | KeyCode::Char('G') => {
                            cursor_line = total_lines.saturating_sub(1);
                            scroll = max_scroll;
                        }
                        _ => {}
                    }
                }
                Mode::Edit => {
                    if key.code == KeyCode::Esc {
                        let text = textarea_content(&textarea);
                        let editor_line = textarea.cursor().0;
                        let visible_height = terminal.size()?.height.saturating_sub(2) as usize;
                        render = render_markdown(&text, MAX_WIDTH);

                        cursor_line = find_rendered_line(&render.source_map, editor_line);

                        // Estimate where the cursor was on screen in the editor.
                        // The textarea scrolls so the cursor is always visible,
                        // so the cursor's screen row is at most visible_height-1.
                        // Approximate: if editor_line > visible_height, cursor was
                        // likely in the middle-ish of the screen.
                        let screen_row = if editor_line < visible_height {
                            editor_line
                        } else {
                            visible_height / 3
                        };

                        scroll = (cursor_line as usize).saturating_sub(screen_row) as u16;

                        mode = Mode::View;
                        continue;
                    }

                    if key.code == KeyCode::Char('s')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        let text = textarea_content(&textarea);
                        fs::write(path, &text)?;
                        render = render_markdown(&text, MAX_WIDTH);
                        modified = false;
                        continue;
                    }

                    let event = Event::Key(key);
                    if textarea.input(event) {
                        modified = true;
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

/// Find the rendered line index that best matches a given source line number.
fn find_rendered_line(source_map: &[usize], source_line: usize) -> u16 {
    // Find first rendered line that maps to >= source_line
    for (i, &src) in source_map.iter().enumerate() {
        if src >= source_line {
            return i as u16;
        }
    }
    source_map.len().saturating_sub(1) as u16
}

fn textarea_content(textarea: &TextArea) -> String {
    let lines = textarea.lines();
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn render_markdown(input: &str, max_width: usize) -> RenderResult {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let code_width = max_width - CODE_MARGIN;

    // Pre-compute byte offset → source line mapping
    let byte_to_line = {
        let mut map = Vec::new();
        let mut line_num = 0usize;
        for (i, ch) in input.char_indices() {
            while map.len() <= i {
                map.push(line_num);
            }
            if ch == '\n' {
                line_num += 1;
            }
        }
        // Pad to cover the full length
        while map.len() <= input.len() {
            map.push(line_num);
        }
        map
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut source_map: Vec<usize> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut list_depth: usize = 0;
    let mut list_indices: Vec<Option<u64>> = Vec::new();
    let mut in_code_block = false;
    let mut code_block_buf = String::new();
    let mut heading_level = HeadingLevel::H1;

    let mut table_row: Vec<String> = Vec::new();
    let mut table_header: Vec<String> = Vec::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut is_header_row = false;
    let mut current_cell = String::new();
    let mut in_cell = false;

    // Track the current source line based on parser offsets
    let mut current_source_line: usize = 0;

    // Use offset iter to track positions
    let parser_with_offsets: Vec<_> = {
        let parser = Parser::new_ext(input, options);
        parser.into_offset_iter().collect()
    };

    for (event, range) in parser_with_offsets {
        // Update current source line from the byte offset
        if let Some(&line) = byte_to_line.get(range.start) {
            current_source_line = line;
        }

        match event {
            MdEvent::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    heading_level = level;
                }
                Tag::Paragraph => {}
                Tag::Emphasis => {
                    let current = current_style(&style_stack);
                    style_stack.push(current.add_modifier(Modifier::ITALIC));
                }
                Tag::Strong => {
                    let current = current_style(&style_stack);
                    style_stack.push(current.add_modifier(Modifier::BOLD));
                }
                Tag::Strikethrough => {
                    let current = current_style(&style_stack);
                    style_stack.push(current.add_modifier(Modifier::CROSSED_OUT));
                }
                Tag::CodeBlock(_) => {
                    in_code_block = true;
                    code_block_buf.clear();
                    flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
                }
                Tag::List(start) => {
                    list_depth += 1;
                    list_indices.push(start);
                }
                Tag::Item => {
                    let indent = "  ".repeat(list_depth.saturating_sub(1));
                    let bullet = if let Some(idx) = list_indices.last_mut() {
                        if let Some(n) = idx {
                            let b = format!("{}{}. ", indent, n);
                            *n += 1;
                            b
                        } else {
                            format!("{}• ", indent)
                        }
                    } else {
                        format!("{}• ", indent)
                    };
                    current_spans.push(Span::styled(bullet, Style::default().fg(Color::Cyan)));
                }
                Tag::BlockQuote(_) => {
                    current_spans.push(Span::styled(
                        "│ ",
                        Style::default().fg(Color::DarkGray),
                    ));
                    let current = current_style(&style_stack);
                    style_stack.push(current.fg(Color::Gray));
                }
                Tag::Link { dest_url, .. } => {
                    let current = current_style(&style_stack);
                    style_stack.push(
                        current
                            .fg(Color::Blue)
                            .add_modifier(Modifier::UNDERLINED),
                    );
                    let _ = dest_url;
                }
                Tag::Table(_alignments) => {
                    table_header.clear();
                    table_rows.clear();
                    flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
                }
                Tag::TableHead => {
                    is_header_row = true;
                    table_row.clear();
                }
                Tag::TableRow => {
                    is_header_row = false;
                    table_row.clear();
                }
                Tag::TableCell => {
                    in_cell = true;
                    current_cell.clear();
                }
                _ => {}
            },
            MdEvent::End(tag_end) => match tag_end {
                TagEnd::Heading(_) => {
                    let (color, prefix) = match heading_level {
                        HeadingLevel::H1 => (Color::Magenta, "# "),
                        HeadingLevel::H2 => (Color::Green, "## "),
                        HeadingLevel::H3 => (Color::Yellow, "### "),
                        HeadingLevel::H4 => (Color::Cyan, "#### "),
                        HeadingLevel::H5 => (Color::Blue, "##### "),
                        HeadingLevel::H6 => (Color::DarkGray, "###### "),
                    };
                    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);
                    let mut heading_spans = vec![Span::styled(prefix.to_string(), style)];
                    for span in current_spans.drain(..) {
                        heading_spans.push(Span::styled(span.content.to_string(), style));
                    }
                    lines.push(Line::from(heading_spans));
                    source_map.push(current_source_line);
                    lines.push(Line::from(""));
                    source_map.push(current_source_line);
                }
                TagEnd::Paragraph => {
                    flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
                    lines.push(Line::from(""));
                    source_map.push(current_source_line);
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    let block_style = Style::default().fg(Color::Gray).bg(Color::Rgb(40, 40, 40));
                    let margin = " ".repeat(CODE_MARGIN / 2);
                    let trimmed = code_block_buf.trim_end_matches('\n');
                    let inner_width = code_width;

                    let top_pad = format!("{}{}", margin, " ".repeat(inner_width));
                    lines.push(Line::from(Span::styled(top_pad, block_style)));
                    source_map.push(current_source_line);

                    for code_line in trimmed.split('\n') {
                        let visible_len = code_line.len();
                        let content = if visible_len >= inner_width - 2 {
                            format!(" {}", &code_line[..inner_width - 2])
                        } else {
                            format!(" {}{}", code_line, " ".repeat(inner_width - 1 - visible_len))
                        };
                        let padded = format!("{}{}", margin, content);
                        lines.push(Line::from(Span::styled(padded, block_style)));
                        source_map.push(current_source_line);
                        current_source_line += 1;
                    }

                    let bot_pad = format!("{}{}", margin, " ".repeat(inner_width));
                    lines.push(Line::from(Span::styled(bot_pad, block_style)));
                    source_map.push(current_source_line);
                    lines.push(Line::from(""));
                    source_map.push(current_source_line);
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    list_indices.pop();
                    if list_depth == 0 {
                        lines.push(Line::from(""));
                        source_map.push(current_source_line);
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
                }
                TagEnd::BlockQuote(_) => {
                    style_stack.pop();
                    flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
                }
                TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::Table => {
                    let num_cols = table_header.len();
                    let mut col_widths: Vec<usize> = table_header.iter().map(|c| c.len()).collect();
                    for row in &table_rows {
                        for (i, cell) in row.iter().enumerate() {
                            if i < col_widths.len() {
                                col_widths[i] = col_widths[i].max(cell.len());
                            }
                        }
                    }

                    let border_style = Style::default().fg(Color::DarkGray);
                    let header_style = Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD);
                    let cell_style = Style::default().fg(Color::White);

                    let top: Vec<String> = col_widths.iter().map(|w| "─".repeat(w + 2)).collect();
                    lines.push(Line::from(Span::styled(
                        format!("┌{}┐", top.join("┬")),
                        border_style,
                    )));
                    source_map.push(current_source_line);

                    let mut header_spans: Vec<Span<'static>> = vec![
                        Span::styled("│", border_style),
                    ];
                    for (i, cell) in table_header.iter().enumerate() {
                        let w = col_widths.get(i).copied().unwrap_or(0);
                        header_spans.push(Span::styled(
                            format!(" {:width$} ", cell, width = w),
                            header_style,
                        ));
                        header_spans.push(Span::styled("│", border_style));
                    }
                    lines.push(Line::from(header_spans));
                    source_map.push(current_source_line);

                    let sep: Vec<String> = col_widths.iter().map(|w| "═".repeat(w + 2)).collect();
                    lines.push(Line::from(Span::styled(
                        format!("╞{}╡", sep.join("╪")),
                        border_style,
                    )));
                    source_map.push(current_source_line);

                    for row in &table_rows {
                        let mut row_spans: Vec<Span<'static>> = vec![
                            Span::styled("│", border_style),
                        ];
                        for (i, cell) in row.iter().enumerate() {
                            let w = col_widths.get(i).copied().unwrap_or(0);
                            row_spans.push(Span::styled(
                                format!(" {:width$} ", cell, width = w),
                                cell_style,
                            ));
                            row_spans.push(Span::styled("│", border_style));
                        }
                        for i in row.len()..num_cols {
                            let w = col_widths.get(i).copied().unwrap_or(0);
                            row_spans.push(Span::styled(
                                format!(" {:width$} ", "", width = w),
                                cell_style,
                            ));
                            row_spans.push(Span::styled("│", border_style));
                        }
                        lines.push(Line::from(row_spans));
                        source_map.push(current_source_line);
                        current_source_line += 1;
                    }

                    let bot: Vec<String> = col_widths.iter().map(|w| "─".repeat(w + 2)).collect();
                    lines.push(Line::from(Span::styled(
                        format!("└{}┘", bot.join("┴")),
                        border_style,
                    )));
                    source_map.push(current_source_line);
                    lines.push(Line::from(""));
                    source_map.push(current_source_line);
                }
                TagEnd::TableHead => {
                    table_header = table_row.clone();
                    table_row.clear();
                }
                TagEnd::TableRow => {
                    if !is_header_row {
                        table_rows.push(table_row.clone());
                    }
                    table_row.clear();
                }
                TagEnd::TableCell => {
                    in_cell = false;
                    table_row.push(current_cell.clone());
                }
                _ => {}
            },
            MdEvent::Text(text) => {
                if in_code_block {
                    code_block_buf.push_str(&text);
                } else if in_cell {
                    current_cell.push_str(&text);
                } else {
                    let style = current_style(&style_stack);
                    current_spans.push(Span::styled(text.to_string(), style));
                }
            }
            MdEvent::Code(code) => {
                if in_cell {
                    current_cell.push_str(&format!("`{}`", code));
                } else {
                    current_spans.push(Span::styled(
                        format!(" {} ", code),
                        Style::default().fg(Color::Yellow).bg(Color::Rgb(40, 40, 40)),
                    ));
                }
            }
            MdEvent::SoftBreak => {
                if !in_code_block {
                    current_spans.push(Span::raw(" "));
                }
            }
            MdEvent::HardBreak => {
                flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
            }
            MdEvent::Rule => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(max_width),
                    Style::default().fg(Color::DarkGray),
                )));
                source_map.push(current_source_line);
                lines.push(Line::from(""));
                source_map.push(current_source_line);
            }
            MdEvent::TaskListMarker(checked) => {
                let marker = if checked { "☑ " } else { "☐ " };
                current_spans.push(Span::styled(
                    marker.to_string(),
                    Style::default().fg(Color::Cyan),
                ));
            }
            _ => {}
        }
    }

    flush_line(&mut lines, &mut source_map, &mut current_spans, max_width, current_source_line);
    RenderResult { lines, source_map }
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(
    lines: &mut Vec<Line<'static>>,
    source_map: &mut Vec<usize>,
    spans: &mut Vec<Span<'static>>,
    max_width: usize,
    source_line: usize,
) {
    if spans.is_empty() {
        return;
    }

    let total_len: usize = spans.iter().map(|s| s.content.len()).sum();

    if total_len <= max_width {
        lines.push(Line::from(spans.drain(..).collect::<Vec<_>>()));
        source_map.push(source_line);
        return;
    }

    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut current_len: usize = 0;

    for span in spans.drain(..) {
        let style = span.style;
        let text = span.content.to_string();

        for word in WordSplitter::new(&text) {
            let word_len = word.len();

            if current_len + word_len > max_width && current_len > 0 {
                lines.push(Line::from(
                    current_line.drain(..).collect::<Vec<_>>(),
                ));
                source_map.push(source_line);
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
        lines.push(Line::from(current_line));
        source_map.push(source_line);
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
