use std::env;
use std::fs;
use std::io::{self, stdout};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
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

const MAX_WIDTH: usize = 80;
const CODE_MARGIN: usize = 4;

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

    let lines = render_markdown(&content, MAX_WIDTH);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut scroll: u16 = 0;
    let total_lines = lines.len() as u16;

    loop {
        terminal.draw(|frame| {
            let area = frame.area();

            let chunks = Layout::horizontal([
                Constraint::Min(0),
                Constraint::Length(1),
            ])
            .split(area);

            let visible_height = chunks[0].height.saturating_sub(2);
            let max_scroll = total_lines.saturating_sub(visible_height);

            let block = Block::default()
                .title(format!(" {} ", path))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray));

            let paragraph = Paragraph::new(lines.clone())
                .block(block)
                .scroll((scroll, 0));

            frame.render_widget(paragraph, chunks[0]);

            let mut scrollbar_state =
                ScrollbarState::new(max_scroll as usize).position(scroll as usize);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            frame.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);
        })?;

        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            let visible_height = terminal.size()?.height.saturating_sub(2);
            let max_scroll = total_lines.saturating_sub(visible_height);

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Down | KeyCode::Char('j') => {
                    scroll = scroll.saturating_add(1).min(max_scroll);
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    scroll = scroll.saturating_sub(1);
                }
                KeyCode::PageDown | KeyCode::Char(' ') => {
                    scroll = scroll.saturating_add(visible_height).min(max_scroll);
                }
                KeyCode::PageUp => {
                    scroll = scroll.saturating_sub(visible_height);
                }
                KeyCode::Home | KeyCode::Char('g') => {
                    scroll = 0;
                }
                KeyCode::End | KeyCode::Char('G') => {
                    scroll = max_scroll;
                }
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn render_markdown(input: &str, max_width: usize) -> Vec<Line<'static>> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(input, options);

    let code_width = max_width - CODE_MARGIN;

    let mut lines: Vec<Line<'static>> = Vec::new();
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

    for event in parser {
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
                    flush_line(&mut lines, &mut current_spans, max_width);
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
                    flush_line(&mut lines, &mut current_spans, max_width);
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
                    lines.push(Line::from(""));
                }
                TagEnd::Paragraph => {
                    flush_line(&mut lines, &mut current_spans, max_width);
                    lines.push(Line::from(""));
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

                    // Empty line with background for top padding
                    let top_pad = format!("{}{}", margin, " ".repeat(inner_width));
                    lines.push(Line::from(Span::styled(top_pad, block_style)));

                    for code_line in trimmed.split('\n') {
                        let visible_len = code_line.len();
                        let content = if visible_len >= inner_width - 2 {
                            format!(" {}", &code_line[..inner_width - 2])
                        } else {
                            format!(" {}{}", code_line, " ".repeat(inner_width - 1 - visible_len))
                        };
                        let padded = format!("{}{}", margin, content);
                        lines.push(Line::from(Span::styled(padded, block_style)));
                    }

                    // Empty line with background for bottom padding
                    let bot_pad = format!("{}{}", margin, " ".repeat(inner_width));
                    lines.push(Line::from(Span::styled(bot_pad, block_style)));
                    lines.push(Line::from(""));
                }
                TagEnd::List(_) => {
                    list_depth = list_depth.saturating_sub(1);
                    list_indices.pop();
                    if list_depth == 0 {
                        lines.push(Line::from(""));
                    }
                }
                TagEnd::Item => {
                    flush_line(&mut lines, &mut current_spans, max_width);
                }
                TagEnd::BlockQuote(_) => {
                    style_stack.pop();
                    flush_line(&mut lines, &mut current_spans, max_width);
                }
                TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::Table => {

                    // Compute column widths
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

                    // Top border
                    let top: Vec<String> = col_widths.iter().map(|w| "─".repeat(w + 2)).collect();
                    lines.push(Line::from(Span::styled(
                        format!("┌{}┐", top.join("┬")),
                        border_style,
                    )));

                    // Header row
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

                    // Header separator
                    let sep: Vec<String> = col_widths.iter().map(|w| "═".repeat(w + 2)).collect();
                    lines.push(Line::from(Span::styled(
                        format!("╞{}╡", sep.join("╪")),
                        border_style,
                    )));

                    // Data rows
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
                        // Pad missing cells
                        for i in row.len()..num_cols {
                            let w = col_widths.get(i).copied().unwrap_or(0);
                            row_spans.push(Span::styled(
                                format!(" {:width$} ", "", width = w),
                                cell_style,
                            ));
                            row_spans.push(Span::styled("│", border_style));
                        }
                        lines.push(Line::from(row_spans));
                    }

                    // Bottom border
                    let bot: Vec<String> = col_widths.iter().map(|w| "─".repeat(w + 2)).collect();
                    lines.push(Line::from(Span::styled(
                        format!("└{}┘", bot.join("┴")),
                        border_style,
                    )));
                    lines.push(Line::from(""));
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
                flush_line(&mut lines, &mut current_spans, max_width);
            }
            MdEvent::Rule => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(max_width),
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::from(""));
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

    flush_line(&mut lines, &mut current_spans, max_width);
    lines
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>, max_width: usize) {
    if spans.is_empty() {
        return;
    }

    // Calculate total text length
    let total_len: usize = spans.iter().map(|s| s.content.len()).sum();

    if total_len <= max_width {
        lines.push(Line::from(spans.drain(..).collect::<Vec<_>>()));
        return;
    }

    // Word-wrap: break spans across multiple lines
    let mut current_line: Vec<Span<'static>> = Vec::new();
    let mut current_len: usize = 0;

    for span in spans.drain(..) {
        let style = span.style;
        let text = span.content.to_string();

        for word in WordSplitter::new(&text) {
            let word_len = word.len();

            if current_len + word_len > max_width && current_len > 0 {
                // Emit current line, start new one
                lines.push(Line::from(
                    current_line.drain(..).collect::<Vec<_>>(),
                ));
                current_len = 0;
                // Skip leading space on new line
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
    }
}

/// Splits text into chunks that keep words together with their trailing space.
/// E.g., "hello world foo" -> ["hello ", "world ", "foo"]
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

        // Find end of next word + trailing whitespace
        let bytes = self.remaining.as_bytes();
        let mut i = 0;

        // Skip to end of non-space chars
        while i < bytes.len() && bytes[i] != b' ' {
            i += 1;
        }
        // Include trailing spaces
        while i < bytes.len() && bytes[i] == b' ' {
            i += 1;
        }

        if i == 0 {
            // All spaces
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

