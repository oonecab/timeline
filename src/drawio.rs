//! Read-only projection into native, editable draw.io cells. No database I/O.

use crate::export_graph::ExportGraph;
use crate::{Result, TLNode, Timeline};
use std::fmt::Write;

const WIDTH: usize = 220;
const HEIGHT: usize = 104;
const COLUMN: usize = 290;
const ROW: usize = 156;

/// An uncompressed XML snapshot, independent of the source timeline after export.
pub struct Diagram {
    pub xml: String,
    pub nodes: usize,
    pub edges: usize,
}

#[derive(Clone, Copy, Default)]
struct Position {
    depth: usize,
    row: usize,
}

impl Position {
    fn x(self) -> usize {
        40 + self.depth * COLUMN
    }

    fn y(self) -> usize {
        140 + self.row * ROW
    }
}

/// Render a complete timeline (hot and archived nodes), without changing it.
pub fn render(timeline: &Timeline) -> Result<Diagram> {
    let graph = ExportGraph::new(timeline)?;
    let positions = layout(&graph);

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<mxfile host=\"Timeline\" compressed=\"false\">\n",
    );
    writeln!(
        xml,
        "  <diagram id=\"{}\" name=\"{}\">",
        attr(&timeline.id),
        attr(&timeline.id)
    )
    .unwrap();
    xml.push_str("    <mxGraphModel grid=\"0\" gridSize=\"10\" guides=\"1\" tooltips=\"1\" connect=\"1\" arrows=\"1\" fold=\"1\" page=\"0\" background=\"#ffffff\" math=\"0\" shadow=\"0\">\n      <root>\n        <mxCell id=\"0\"/>\n        <mxCell id=\"1\" value=\"历史节点与父子关系\" parent=\"0\"/>\n        <mxCell id=\"coverage\" value=\"摘要覆盖（非历史父子关系）\" parent=\"0\" visible=\"0\"/>\n        <mxCell id=\"versions\" value=\"摘要版本与依赖（非分支合并）\" parent=\"0\" visible=\"0\"/>\n");
    let legend = format!(
        "Timeline {} · 当前分支 {}\n实线箭头：历史父子关系；绿色：当前 head；灰色：冷区；PIN：保留\n摘要覆盖 / 版本 / 依赖位于独立图层（默认隐藏）。这是离线快照，改图不回写 Timeline。",
        timeline.id,
        preview(&timeline.current_branch, 48)
    );
    writeln!(xml, "        <mxCell id=\"legend\" value=\"{}\" style=\"text;html=0;align=left;verticalAlign=middle;whiteSpace=wrap;fontSize=13;fontColor=#536674;\" vertex=\"1\" parent=\"1\"><mxGeometry x=\"40\" y=\"24\" width=\"920\" height=\"90\" as=\"geometry\"/></mxCell>", attr(&legend)).unwrap();

    for (i, node) in graph.nodes.iter().enumerate() {
        let branch_names = graph
            .heads
            .get(node.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        write_node(
            &mut xml,
            timeline,
            node,
            positions[i],
            branch_names,
            node.id == graph.head,
        );
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        if let Some(parent) = &node.parent {
            let a = positions[graph.indices[parent.as_str()]];
            let b = positions[i];
            // A shared junction in the column gap keeps elbows outside all boxes.
            let via_x = a.x() + WIDTH + (COLUMN - WIDTH) / 2;
            let points = [(via_x, a.y() + HEIGHT / 2), (via_x, b.y() + HEIGHT / 2)];
            write_edge(
                &mut xml,
                &format!("parent-{}", node.id),
                parent,
                &node.id,
                "1",
                "",
                "parent",
                &points,
            );
        }
        if let Some(range) = &node.summary_range {
            for (kind, endpoint) in [("coverage-from", &range.from), ("coverage-to", &range.to)] {
                if kind == "coverage-to" && range.from == range.to {
                    continue;
                }
                let label = if range.from == range.to {
                    "覆盖"
                } else if kind == "coverage-from" {
                    "覆盖起点"
                } else {
                    "覆盖终点"
                };
                write_edge(
                    &mut xml,
                    &format!("{kind}-{}", node.id),
                    &node.id,
                    endpoint,
                    "coverage",
                    label,
                    kind,
                    &[],
                );
            }
        }
        if let Some(old) = &node.supersedes {
            write_edge(
                &mut xml,
                &format!("supersedes-{}", node.id),
                &node.id,
                old,
                "versions",
                "替代旧版",
                "supersedes",
                &[],
            );
        }
        for (j, dependency) in node.derived_from.iter().enumerate() {
            write_edge(
                &mut xml,
                &format!("derived-{}-{j}", node.id),
                &node.id,
                dependency,
                "versions",
                "参考摘要",
                "derived_from",
                &[],
            );
        }
    }
    xml.push_str("      </root>\n    </mxGraphModel>\n  </diagram>\n</mxfile>\n");
    Ok(Diagram {
        xml,
        nodes: graph.nodes.len(),
        edges: graph.edges,
    })
}

// Iterative tree layout: current path first, then other history, then summaries.
// First child continues the parent's row; each sibling subtree gets separate rows.
// This avoids recursion limits and keeps long, unbranched conversations linear.
fn layout(graph: &ExportGraph<'_>) -> Vec<Position> {
    let children = &graph.children;
    let mut positions = vec![Position::default(); graph.nodes.len()];
    let mut stack = vec![(graph.root, 0, false)];
    let mut next_row = 0;
    while let Some((i, depth, exiting)) = stack.pop() {
        if exiting {
            positions[i] = Position {
                depth,
                row: positions[children[i][0]].row,
            };
        } else {
            if children[i].is_empty() {
                positions[i] = Position {
                    depth,
                    row: next_row,
                };
                next_row += 1;
            } else {
                stack.push((i, depth, true));
                for &child in children[i].iter().rev() {
                    stack.push((child, depth + 1, false));
                }
            }
        }
    }
    positions
}

fn write_node(
    xml: &mut String,
    timeline: &Timeline,
    node: &TLNode,
    position: Position,
    branches: &[&str],
    current: bool,
) {
    let cold = timeline.archive.contains_key(&node.id);
    let role = node.message.as_ref().map_or("root", |m| m.role.as_str());
    let text = node.message.as_ref().map_or("", |m| m.text.as_str());
    let tokens = node.message.as_ref().map_or(0, |m| m.tokens);
    let mut status = match role {
        "root" => "起点".to_owned(),
        "user" => "用户".to_owned(),
        "assistant" => "助手".to_owned(),
        "system" => "系统".to_owned(),
        "summary" => format!("摘要 v{}", node.summary_version),
        _ => role.to_owned(),
    };
    if node.pinned {
        status.push_str(" · PIN");
    }
    if cold {
        status.push_str(" · 冷区");
    }
    if node.is_placeholder() {
        status.push_str(" · 待回填");
    }
    if node.stale {
        status.push_str(" · STALE");
    }
    let mut label = format!("{} · {}\n{}", node.id, status, preview(text, 52));
    if let Some(range) = &node.summary_range {
        write!(label, "\n覆盖 {} → {}", range.from, range.to).unwrap();
    }
    if current {
        write!(
            label,
            "\n{} · 当前 head",
            preview(&timeline.current_branch, 24)
        )
        .unwrap();
    } else if !branches.is_empty() {
        write!(label, "\n{} · head", preview(&branches.join(", "), 28)).unwrap();
    }
    let (fill, stroke, ink) = if current {
        ("#E8F4EC", "#82AC96", "#28563D")
    } else if cold {
        ("#F1F3F5", "#CDD3D8", "#63717A")
    } else if node.summary_range.is_some() {
        ("#F1EFFA", "#D4CEE7", "#5D507F")
    } else {
        ("#E5F2FF", "#D8E2E8", "#12528E")
    };
    let values = [
        ("id", node.id.clone()),
        ("label", label),
        ("timeline_id", timeline.id.clone()),
        ("node_id", node.id.clone()),
        ("role", role.to_owned()),
        // JSON retains even characters that XML 1.0 cannot directly represent.
        ("text_json", serde_json::to_string(text).unwrap()),
        ("tokens", tokens.to_string()),
        ("parent_id", node.parent.clone().unwrap_or_default()),
        (
            "children_json",
            serde_json::to_string(&node.children).unwrap(),
        ),
        ("branches_json", serde_json::to_string(branches).unwrap()),
        ("pinned", node.pinned.to_string()),
        ("zone", if cold { "cold" } else { "hot" }.into()),
        (
            "summary_range_json",
            serde_json::to_string(&node.summary_range).unwrap(),
        ),
        ("summary_version", node.summary_version.to_string()),
        ("supersedes", node.supersedes.clone().unwrap_or_default()),
        (
            "derived_from_json",
            serde_json::to_string(&node.derived_from).unwrap(),
        ),
        ("stale", node.stale.to_string()),
        ("placeholder", node.is_placeholder().to_string()),
        ("current_head", current.to_string()),
        ("placeholders", "0".into()),
        // Keep tooltips plain and bounded; full conversation text is Edit Data.
        (
            "tooltip",
            format!("{} · {} · {} tokens", node.id, role, tokens),
        ),
    ];
    xml.push_str("        <UserObject");
    for (key, value) in values {
        write!(xml, " {key}=\"{}\"", attr(&value)).unwrap();
    }
    writeln!(xml, "><mxCell style=\"rounded=1;arcSize=18;whiteSpace=wrap;html=0;align=center;verticalAlign=middle;spacing=10;fontSize=13;fontFamily=Helvetica;fillColor={fill};strokeColor={stroke};fontColor={ink};\" vertex=\"1\" parent=\"1\"><mxGeometry x=\"{}\" y=\"{}\" width=\"{WIDTH}\" height=\"{HEIGHT}\" as=\"geometry\"/></mxCell></UserObject>", position.x(), position.y()).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn write_edge(
    xml: &mut String,
    id: &str,
    source: &str,
    target: &str,
    layer: &str,
    label: &str,
    relation: &str,
    points: &[(usize, usize)],
) {
    let (color, extra) = if relation == "parent" {
        ("#A3ADB4", "exitX=1;exitY=0.5;entryX=0;entryY=0.5;")
    } else if layer == "coverage" {
        ("#8981A5", "dashed=1;dashPattern=4 4;endArrow=open;")
    } else {
        ("#AE9569", "dashed=1;dashPattern=2 4;endArrow=open;")
    };
    writeln!(xml, "        <UserObject id=\"{}\" label=\"{}\" relation=\"{relation}\"><mxCell style=\"edgeStyle=orthogonalEdgeStyle;rounded=1;orthogonalLoop=1;jettySize=auto;html=0;endArrow=classic;endFill=1;strokeWidth=1;strokeColor={color};fontSize=11;fontColor={color};labelBackgroundColor=#ffffff;{extra}\" edge=\"1\" parent=\"{layer}\" source=\"{}\" target=\"{}\"><mxGeometry relative=\"1\" as=\"geometry\">", attr(id), attr(label), attr(source), attr(target)).unwrap();
    if !points.is_empty() {
        xml.push_str("          <Array as=\"points\">");
        for (x, y) in points {
            write!(xml, "<mxPoint x=\"{x}\" y=\"{y}\"/>").unwrap();
        }
        xml.push_str("</Array>\n");
    }
    xml.push_str("        </mxGeometry></mxCell></UserObject>\n");
}

fn preview(text: &str, limit: usize) -> String {
    let mut out = String::new();
    let mut chars = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c });
    for c in chars.by_ref().take(limit) {
        out.push(c);
    }
    if chars.next().is_some() {
        out.push('…');
    }
    out
}

fn attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}' => {
                out.push(c)
            }
            _ => write!(out, "\\u{:04x}", c as u32).unwrap(),
        }
    }
    out
}
