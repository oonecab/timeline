//! Terminal-readable, read-only timeline snapshots. No ANSI or Unicode box drawing.

use crate::export_graph::ExportGraph;
use crate::{Result, Timeline};
use std::fmt::Write;

pub struct Diagram {
    pub text: String,
    pub nodes: usize,
    pub edges: usize,
}

/// Render every hot/cold node once. Only forks increase indentation, not chains.
pub fn render(timeline: &Timeline) -> Result<Diagram> {
    let graph = ExportGraph::new(timeline)?;
    let mut text = format!(
        "Timeline {} | current={}\nHistory: parent links | HEAD*: current branch | COLD: archived\nSummary links are annotations, not merges.\n\n",
        timeline.id,
        preview(&timeline.current_branch)
    );
    // An explicit stack keeps long conversation chains off the call stack.
    let mut stack = vec![(graph.root, String::new(), "")];
    while let Some((i, prefix, connector)) = stack.pop() {
        let node = graph.nodes[i];
        let role = node.message.as_ref().map_or("root", |m| m.role.as_str());
        write!(text, "{prefix}{connector}[{} {role}", node.id).unwrap();
        if node.summary_range.is_some() {
            write!(text, " v{}", node.summary_version).unwrap();
        }
        for (enabled, tag) in [
            (node.pinned, "PIN"),
            (timeline.archive.contains_key(&node.id), "COLD"),
            (node.is_placeholder(), "PLACEHOLDER"),
            (node.stale, "STALE"),
        ] {
            if enabled {
                write!(text, " {tag}").unwrap();
            }
        }
        text.push(']');
        if let Some(message) = &node.message {
            write!(text, " {}", preview(&message.text)).unwrap();
        }
        if let Some(names) = graph.heads.get(node.id.as_str()) {
            for name in names {
                let marker = if *name == timeline.current_branch {
                    "HEAD*"
                } else {
                    "HEAD"
                };
                write!(text, " [{marker} {}]", preview(name)).unwrap();
            }
        }
        text.push('\n');
        let body = match connector {
            "+-- " => format!("{prefix}|   "),
            "`-- " => format!("{prefix}    "),
            _ => prefix,
        };
        if let Some(range) = &node.summary_range {
            write!(text, "{body}  covers={}..{}", range.from, range.to).unwrap();
            if let Some(old) = &node.supersedes {
                write!(text, " supersedes={old}").unwrap();
            }
            if !node.derived_from.is_empty() {
                write!(text, " derived_from={}", node.derived_from.join(",")).unwrap();
            }
            text.push('\n');
        }
        let children = &graph.children[i];
        if !children.is_empty() {
            writeln!(text, "{body}|").unwrap();
        }
        if children.len() == 1 {
            stack.push((children[0], body, ""));
        } else {
            for (j, &child) in children.iter().enumerate().rev() {
                let connector = if j + 1 == children.len() {
                    "`-- "
                } else {
                    "+-- "
                };
                stack.push((child, body.clone(), connector));
            }
        }
    }
    Ok(Diagram {
        text,
        nodes: graph.nodes.len(),
        edges: graph.edges,
    })
}

// Keep user text on one line, including terminal control sequences and bidi marks.
// Content stays UTF-8; only the drawing syntax is restricted to ASCII.
fn preview(value: &str) -> String {
    let mut out = String::from("\"");
    let mut chars = value.chars();
    for c in chars.by_ref().take(64) {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control()
                || matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{fffe}' | '\u{ffff}') =>
            {
                write!(out, "\\u{{{:x}}}", c as u32).unwrap();
            }
            _ => out.push(c),
        }
    }
    if chars.next().is_some() {
        out.push_str("...");
    }
    out.push('"');
    out
}
