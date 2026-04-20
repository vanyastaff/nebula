//! TUI rendering — draws the execution detail view.
//!
//! Layout (matching user's mockup):
//! ┌─ Header: workflow name, status badge, elapsed ──────────────────────┐
//! │ ┌─ NODE GRAPH ──────────────┐ ┌─ CURRENT NODE ───────────────────┐ │
//! │ │  Node list with status    │ │  Detail for selected node        │ │
//! │ │  icons and durations      │ │  (action, status, output/error)  │ │
//! │ └───────────────────────────┘ └──────────────────────────────────┘ │
//! │ ┌─ EXECUTION LOG ──────────────────────────────────────────────────┐│
//! │ │  Timestamped log entries with colored levels                     ││
//! │ └─────────────────────────────────────────────────────────────────┘│
//! │ keybindings footer                                                  │
//! └─────────────────────────────────────────────────────────────────────┘

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table, Wrap},
};

use super::{
    app::{App, LogEntry, NodeStatus},
    event::LogLevel,
};

pub(crate) fn draw(frame: &mut Frame, app: &App) {
    let outer = Layout::vertical([
        Constraint::Length(2), // header
        Constraint::Min(8),    // main area
        Constraint::Length(8), // log panel
        Constraint::Length(1), // progress bar
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    draw_header(frame, outer[0], app);

    // Main area: node graph (left) + detail panel (right)
    let main = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(outer[1]);

    draw_node_graph(frame, main[0], app);
    draw_detail_panel(frame, main[1], app);
    draw_log_panel(frame, outer[2], app);
    draw_progress(frame, outer[3], app);
    draw_footer(frame, outer[4], app);
}

fn draw_header(frame: &mut Frame, area: Rect, app: &App) {
    let elapsed = format_duration(app.total_elapsed());

    let status_badge = if app.done {
        if app.success {
            Span::styled(
                " ● completed ",
                Style::default().fg(Color::Black).bg(Color::Green),
            )
        } else {
            Span::styled(
                " ● failed ",
                Style::default().fg(Color::White).bg(Color::Red),
            )
        }
    } else {
        let running = app
            .nodes
            .iter()
            .filter(|(_, n)| n.status == NodeStatus::Running)
            .count();
        Span::styled(
            format!(" ● running {running}/{} ", app.nodes.len()),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        )
    };

    let header = Line::from(vec![
        Span::styled(
            format!(" {} ", app.workflow_name),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        status_badge,
        Span::raw("  "),
        Span::styled(
            format!("started · elapsed {elapsed}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let exec_line = Line::from(vec![Span::styled(
        format!(" execution {}", app.execution_id),
        Style::default().fg(Color::DarkGray),
    )]);

    let paragraph = Paragraph::new(vec![header, exec_line]);
    frame.render_widget(paragraph, area);
}

fn draw_node_graph(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" NODE GRAPH ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let rows: Vec<Row> = app
        .nodes
        .iter()
        .enumerate()
        .map(|(i, (_, info))| {
            let is_selected = i == app.selected_node;
            let status_cell = status_cell(&info.status);
            let duration = info.elapsed.map_or_else(|| "—".to_owned(), format_duration);

            let name_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let row = Row::new(vec![
                Cell::from(Span::styled(&info.name, name_style)),
                status_cell,
                Cell::from(Span::styled(duration, Style::default().fg(Color::DarkGray))),
            ]);

            if is_selected {
                row.style(Style::default().bg(Color::Rgb(30, 40, 50)))
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Min(14),
        Constraint::Length(10),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths).block(block).header(
        Row::new(vec!["Node", "Status", "Time"])
            .style(Style::default().fg(Color::DarkGray))
            .bottom_margin(0),
    );

    frame.render_widget(table, area);
}

fn draw_detail_panel(frame: &mut Frame, area: Rect, app: &App) {
    let Some(info) = app.selected_info() else {
        let block = Block::default()
            .title(" DETAIL ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        frame.render_widget(block, area);
        return;
    };

    // Split detail panel: top for node info, bottom for error/output
    let chunks = Layout::vertical([
        Constraint::Length(6), // node info
        Constraint::Min(3),    // output or error
    ])
    .split(area);

    // Node info section
    let info_block = Block::default()
        .title(format!(" CURRENT NODE — {} ", info.name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let status_str = status_text(&info.status).to_owned();
    let duration_str = info.elapsed.map_or_else(|| "—".to_owned(), format_duration);

    let info_lines = vec![
        info_line("action", &info.action_key, Color::White),
        info_line("status", &status_str, status_color(&info.status)),
        info_line("duration", &duration_str, Color::White),
    ];

    let info_para = Paragraph::new(info_lines).block(info_block);
    frame.render_widget(info_para, chunks[0]);

    // Output or error section
    if let Some(error) = &info.error {
        let err_block = Block::default()
            .title(" ERROR DETAIL ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let err_para = Paragraph::new(Span::styled(
            error.as_str(),
            Style::default().fg(Color::Red),
        ))
        .block(err_block)
        .wrap(Wrap { trim: false });
        frame.render_widget(err_para, chunks[1]);
    } else if let Some(output) = &info.output {
        let out_block = Block::default()
            .title(" OUTPUT ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let json = serde_json::to_string_pretty(output).unwrap_or_else(|_| "???".to_owned());
        let out_para = Paragraph::new(json)
            .block(out_block)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(Color::Green));
        frame.render_widget(out_para, chunks[1]);
    } else {
        let block = Block::default()
            .title(" OUTPUT ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let para = Paragraph::new(Span::styled(
            "(pending)",
            Style::default().fg(Color::DarkGray),
        ))
        .block(block);
        frame.render_widget(para, chunks[1]);
    }
}

fn draw_log_panel(frame: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" EXECUTION LOG ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let lines: Vec<Line> = app
        .logs
        .iter()
        .rev()
        .take(20)
        .rev()
        .map(format_log_line)
        .collect();

    let para = Paragraph::new(lines).block(block);
    frame.render_widget(para, area);
}

fn draw_progress(frame: &mut Frame, area: Rect, app: &App) {
    let total = app.nodes.len();
    let done = app.completed_count() + app.failed_count();
    let ratio = if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    };

    let label = format!("{done}/{total} nodes");
    let color = if app.failed_count() > 0 {
        Color::Red
    } else if app.done {
        Color::Green
    } else {
        Color::Yellow
    };

    let gauge = Gauge::default()
        .ratio(ratio)
        .label(label)
        .gauge_style(Style::default().fg(color));

    frame.render_widget(gauge, area);
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App) {
    let keys = if app.done {
        " q quit · ↑↓ select node · i inspect output"
    } else {
        " q quit · ↑↓ select node · k kill"
    };

    let exec_id = format!("execution {}", &app.execution_id[..8]);

    let footer = Line::from(vec![
        Span::styled(keys, Style::default().fg(Color::DarkGray)),
        Span::raw("    "),
        Span::styled(exec_id, Style::default().fg(Color::DarkGray)),
    ]);

    frame.render_widget(Paragraph::new(footer), area);
}

// ── Helpers ──────────────────────────────────────────────────────────────

fn status_cell(status: &NodeStatus) -> Cell<'static> {
    let (icon, color) = match status {
        NodeStatus::Pending => ("○ pending", Color::DarkGray),
        NodeStatus::Running => ("◉ running", Color::Yellow),
        NodeStatus::Completed => ("✓ done", Color::Green),
        NodeStatus::Failed => ("✗ failed", Color::Red),
        NodeStatus::Skipped => ("⊘ skipped", Color::DarkGray),
    };
    Cell::from(Span::styled(icon.to_owned(), Style::default().fg(color)))
}

fn status_text(status: &NodeStatus) -> &'static str {
    match status {
        NodeStatus::Pending => "pending",
        NodeStatus::Running => "running",
        NodeStatus::Completed => "done",
        NodeStatus::Failed => "failed",
        NodeStatus::Skipped => "skipped",
    }
}

fn status_color(status: &NodeStatus) -> Color {
    match status {
        NodeStatus::Pending | NodeStatus::Skipped => Color::DarkGray,
        NodeStatus::Running => Color::Yellow,
        NodeStatus::Completed => Color::Green,
        NodeStatus::Failed => Color::Red,
    }
}

fn info_line<'a>(label: &'a str, value: &'a str, color: Color) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(color)),
    ])
}

fn format_log_line(entry: &LogEntry) -> Line<'static> {
    let ts = format!("+{:.1}s", entry.offset.as_secs_f64());

    let (level_str, level_color) = match entry.level {
        LogLevel::Info => ("INFO ", Color::Cyan),
        LogLevel::Warn => ("WARN ", Color::Yellow),
        LogLevel::Error => ("ERROR", Color::Red),
    };

    Line::from(vec![
        Span::styled(format!("{ts:<8}"), Style::default().fg(Color::DarkGray)),
        Span::styled(level_str, Style::default().fg(level_color)),
        Span::raw(" "),
        Span::styled(entry.message.clone(), Style::default().fg(Color::White)),
    ])
}

fn format_duration(d: std::time::Duration) -> String {
    if d.as_millis() < 1000 {
        format!("{}ms", d.as_millis())
    } else if d.as_secs() < 60 {
        format!("{:.1}s", d.as_secs_f64())
    } else {
        format!("{}m{:.0}s", d.as_secs() / 60, d.as_secs() % 60)
    }
}
