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

    let lines = render_markdown(&content);

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

fn render_markdown(input: &str) -> Vec<Line<'static>> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(input, options);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut style_stack: Vec<Style> = vec![Style::default()];
    let mut list_depth: usize = 0;
    let mut list_indices: Vec<Option<u64>> = Vec::new();
    let mut in_code_block = false;
    let mut code_block_lines: Vec<String> = Vec::new();
    let mut heading_level = HeadingLevel::H1;
    let mut in_table = false;
    let mut table_row: Vec<String> = Vec::new();

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
                    code_block_lines.clear();
                    flush_line(&mut lines, &mut current_spans);
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
                    in_table = true;
                    flush_line(&mut lines, &mut current_spans);
                }
                Tag::TableHead => {
                    table_row.clear();
                }
                Tag::TableRow => {
                    table_row.clear();
                }
                Tag::TableCell => {}
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
                    flush_line(&mut lines, &mut current_spans);
                    lines.push(Line::from(""));
                }
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                    style_stack.pop();
                }
                TagEnd::CodeBlock => {
                    in_code_block = false;
                    let block_style = Style::default().fg(Color::White).bg(Color::DarkGray);
                    for code_line in code_block_lines.drain(..) {
                        let padded = format!("  {}  ", code_line);
                        lines.push(Line::from(Span::styled(padded, block_style)));
                    }
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
                    flush_line(&mut lines, &mut current_spans);
                }
                TagEnd::BlockQuote(_) => {
                    style_stack.pop();
                    flush_line(&mut lines, &mut current_spans);
                }
                TagEnd::Link => {
                    style_stack.pop();
                }
                TagEnd::Table => {
                    in_table = false;
                    lines.push(Line::from(""));
                }
                TagEnd::TableHead => {
                    let header_style = Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD);
                    let separator_style = Style::default().fg(Color::DarkGray);

                    let row_str = format_table_row(&table_row);
                    lines.push(Line::from(Span::styled(row_str, header_style)));

                    let sep: Vec<String> = table_row
                        .iter()
                        .map(|cell| "─".repeat(cell.len().max(3) + 2))
                        .collect();
                    lines.push(Line::from(Span::styled(
                        format!("├{}┤", sep.join("┼")),
                        separator_style,
                    )));
                    table_row.clear();
                }
                TagEnd::TableRow => {
                    let row_str = format_table_row(&table_row);
                    lines.push(Line::from(Span::styled(
                        row_str,
                        Style::default().fg(Color::White),
                    )));
                    table_row.clear();
                }
                TagEnd::TableCell => {}
                _ => {}
            },
            MdEvent::Text(text) => {
                if in_code_block {
                    for line in text.lines() {
                        code_block_lines.push(line.to_string());
                    }
                } else if in_table {
                    table_row.push(text.to_string());
                } else {
                    let style = current_style(&style_stack);
                    current_spans.push(Span::styled(text.to_string(), style));
                }
            }
            MdEvent::Code(code) => {
                if in_table {
                    table_row.push(format!("`{}`", code));
                } else {
                    current_spans.push(Span::styled(
                        format!(" {} ", code),
                        Style::default().fg(Color::Yellow).bg(Color::DarkGray),
                    ));
                }
            }
            MdEvent::SoftBreak => {
                if !in_code_block {
                    current_spans.push(Span::raw(" "));
                }
            }
            MdEvent::HardBreak => {
                flush_line(&mut lines, &mut current_spans);
            }
            MdEvent::Rule => {
                lines.push(Line::from(Span::styled(
                    "─".repeat(60),
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

    flush_line(&mut lines, &mut current_spans);
    lines
}

fn current_style(stack: &[Style]) -> Style {
    stack.last().copied().unwrap_or_default()
}

fn flush_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    if !spans.is_empty() {
        lines.push(Line::from(spans.drain(..).collect::<Vec<_>>()));
    }
}

fn format_table_row(cells: &[String]) -> String {
    let padded: Vec<String> = cells.iter().map(|c| format!(" {} ", c)).collect();
    format!("│{}│", padded.join("│"))
}
