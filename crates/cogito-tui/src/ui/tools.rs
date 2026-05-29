//! Tools pane widget — renders per-turn tool tree with expansion.
//!
//! Selection (`selected: Option<TreePath>`) highlights one node;
//! `expanded: &HashSet<TreePath>` toggles inline args+result preview
//! lines under the selected node. The expansion data (result preview)
//! is populated lazily by the App on Ctrl-Enter (spec §5.3, decision
//! α.1) — this widget just renders what's there.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::render_model::{ToolNode, ToolStatus, ToolTreeModel, TreePath};

/// Indent applied to expanded args/result lines beneath a node.
const EXPAND_INDENT: &str = "    ";

/// Max args-preview lines printed under an expanded node.
const EXPAND_ARGS_LINES: usize = 5;

/// Max result-preview chars printed under an expanded node.
const EXPAND_RESULT_CHARS: usize = 800;

/// Render the tools pane into `area`. `selected` is the current
/// selection cursor (None = nothing selected); `expanded` is the set
/// of selected paths whose args+result preview should be displayed.
pub fn render<S: ::std::hash::BuildHasher>(
    f: &mut Frame,
    area: Rect,
    model: &ToolTreeModel,
    selected: Option<TreePath>,
    expanded: &HashSet<TreePath, S>,
) {
    let block = Block::default().borders(Borders::ALL).title("tools");

    let lines: Vec<Line<'static>> = if model.turns.is_empty() {
        vec![Line::from(vec![Span::styled(
            "(no tool calls yet)",
            Style::default().add_modifier(Modifier::DIM),
        )])]
    } else {
        let mut out: Vec<Line<'static>> = Vec::new();
        for (turn_idx, group) in model.turns.iter().enumerate() {
            out.push(Line::from(vec![Span::styled(
                format!("turn {}", group.turn_idx),
                Style::default().add_modifier(Modifier::BOLD),
            )]));
            for (node_idx, node) in group.nodes.iter().enumerate() {
                let path: TreePath = (turn_idx, node_idx);
                let is_selected = selected == Some(path);
                out.push(node_line(node, is_selected));
                if expanded.contains(&path) {
                    out.extend(expansion_lines(node));
                }
            }
        }
        out
    };

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(para, area);
}

fn node_line(node: &ToolNode, is_selected: bool) -> Line<'static> {
    let (status_str, style) = match &node.status {
        ToolStatus::Running => (
            "running".to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::DIM),
        ),
        ToolStatus::Ok { elapsed_ms } => (
            format!("ok ({elapsed_ms}ms)"),
            Style::default().fg(Color::Green),
        ),
        ToolStatus::Err { elapsed_ms, .. } => (
            format!("err ({elapsed_ms}ms)"),
            Style::default().fg(Color::Red),
        ),
    };
    let marker = if is_selected { ">" } else { " " };
    Line::from(vec![
        Span::styled(format!("{marker}  "), Style::default().fg(Color::Cyan)),
        Span::raw(node.tool_name.clone()),
        Span::raw("  "),
        Span::styled(status_str, style),
    ])
}

fn expansion_lines(node: &ToolNode) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    // Args block (pretty JSON, capped).
    let args_pretty =
        serde_json::to_string_pretty(&node.args).unwrap_or_else(|_| "<unencodable>".to_string());
    out.push(Line::from(vec![Span::styled(
        format!("{EXPAND_INDENT}args:"),
        Style::default().add_modifier(Modifier::DIM),
    )]));
    for line in args_pretty.lines().take(EXPAND_ARGS_LINES) {
        out.push(Line::from(vec![Span::raw(format!(
            "{EXPAND_INDENT}{EXPAND_INDENT}{line}"
        ))]));
    }
    if args_pretty.lines().count() > EXPAND_ARGS_LINES {
        out.push(Line::from(vec![Span::styled(
            format!("{EXPAND_INDENT}{EXPAND_INDENT}..."),
            Style::default().add_modifier(Modifier::DIM),
        )]));
    }
    // Result block (lazy: shows '<not yet loaded>' until populated; on
    // error the status's message is shown instead).
    match &node.status {
        ToolStatus::Err { message, .. } if !message.is_empty() => {
            out.push(Line::from(vec![Span::styled(
                format!("{EXPAND_INDENT}error:"),
                Style::default().fg(Color::Red),
            )]));
            for line in message.lines() {
                out.push(Line::from(vec![Span::styled(
                    format!("{EXPAND_INDENT}{EXPAND_INDENT}{line}"),
                    Style::default().fg(Color::Red),
                )]));
            }
        }
        _ => match &node.result_preview {
            Some(preview) => {
                out.push(Line::from(vec![Span::styled(
                    format!("{EXPAND_INDENT}result:"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
                let truncated = if preview.chars().count() > EXPAND_RESULT_CHARS {
                    let head: String = preview.chars().take(EXPAND_RESULT_CHARS).collect();
                    format!("{head}...")
                } else {
                    preview.clone()
                };
                for line in truncated.lines() {
                    out.push(Line::from(vec![Span::raw(format!(
                        "{EXPAND_INDENT}{EXPAND_INDENT}{line}"
                    ))]));
                }
            }
            None => {
                out.push(Line::from(vec![Span::styled(
                    format!("{EXPAND_INDENT}(loading result...)"),
                    Style::default().add_modifier(Modifier::DIM),
                )]));
            }
        },
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    use cogito_protocol::stream::StreamEvent;

    /// Render the tools pane into a `w x h` `TestBackend` terminal and
    /// return the buffer contents as a flat string (one row per line,
    /// separated by newlines). ratatui 0.28 `Buffer` implements `Debug`
    /// but not `Display`, so we read cells directly via `Buffer::content()`.
    fn draw(
        model: &ToolTreeModel,
        selected: Option<TreePath>,
        expanded: &HashSet<TreePath>,
        w: u16,
        h: u16,
    ) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let area = f.area();
                render(f, area, model, selected, expanded);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let width = buf.area().width as usize;
        let cells = buf.content();
        cells
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn empty_model_renders_placeholder() {
        let model = ToolTreeModel::new();
        let out = draw(&model, None, &HashSet::new(), 30, 5);
        assert!(out.contains("(no tool calls yet)"), "got:\n{out}");
    }

    #[test]
    fn single_running_tool_renders_running_marker() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            args: json!({}),
        });
        let out = draw(&model, None, &HashSet::new(), 40, 8);
        assert!(out.contains("turn 1"), "got:\n{out}");
        assert!(out.contains("read_file"), "got:\n{out}");
        assert!(out.contains("running"), "got:\n{out}");
    }

    #[test]
    fn finished_ok_renders_ok_status() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        });
        let out = draw(&model, None, &HashSet::new(), 40, 8);
        assert!(out.contains("ok ("), "got:\n{out}");
    }

    #[test]
    fn selected_node_renders_arrow_marker() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        let out = draw(&model, Some((0, 0)), &HashSet::new(), 40, 8);
        assert!(out.contains(">  t"), "got:\n{out}");
    }

    #[test]
    fn expanded_node_renders_args_and_loading_result() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({"path": "a.rs"}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: true,
            error_message: None,
        });
        let mut expanded = HashSet::new();
        expanded.insert((0, 0));
        let out = draw(&model, Some((0, 0)), &expanded, 50, 14);
        assert!(out.contains("args:"), "got:\n{out}");
        assert!(out.contains("path"), "got:\n{out}");
        assert!(out.contains("(loading result"), "got:\n{out}");
    }

    #[test]
    fn expanded_err_node_shows_error_message() {
        let mut model = ToolTreeModel::new();
        model.on_event(&StreamEvent::TurnStarted);
        model.on_event(&StreamEvent::ToolDispatchStarted {
            call_id: "c1".into(),
            tool_name: "t".into(),
            args: json!({}),
        });
        model.on_event(&StreamEvent::ToolDispatchEnded {
            call_id: "c1".into(),
            ok: false,
            error_message: Some("boom".into()),
        });
        let mut expanded = HashSet::new();
        expanded.insert((0, 0));
        let out = draw(&model, Some((0, 0)), &expanded, 50, 14);
        assert!(out.contains("error:"), "got:\n{out}");
        assert!(out.contains("boom"), "got:\n{out}");
    }

    #[test]
    fn multi_turn_renders_separate_groups() {
        let mut model = ToolTreeModel::new();
        for (i, id) in ["c1", "c2"].iter().enumerate() {
            model.on_event(&StreamEvent::TurnStarted);
            model.on_event(&StreamEvent::ToolDispatchStarted {
                call_id: (*id).into(),
                tool_name: format!("t{i}"),
                args: json!({}),
            });
        }
        let out = draw(&model, None, &HashSet::new(), 40, 10);
        assert!(out.contains("turn 1"), "got:\n{out}");
        assert!(out.contains("turn 2"), "got:\n{out}");
    }
}
