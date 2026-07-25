//! Layered ASCII/Unicode rendering of a workflow DAG.
//!
//! One renderer, two consumers: `spky workflows show` prints the lines to stdout
//! and `spky workflows watch` hands the same lines to ratatui. Keeping it as
//! `Vec<String>` rather than ratatui widgets is what makes the static and live
//! views provably identical.
//!
//! Layout is longest-path layering: a step's column is `1 + max(column of its
//! dependencies)`, so dependencies always sit strictly left of their dependents
//! and a fan-in join lines up after every branch that feeds it. Rows within a
//! column are ordered by the average row of their predecessors (a barycenter
//! pass), which keeps edges from crossing more than they have to. Real workflows
//! are small — tens of steps — so nothing here needs to be cleverer than that.

use std::collections::BTreeMap;

/// A step as the renderer needs it: identity, state, and a one-line detail.
#[derive(Debug, Clone)]
pub struct DagNode {
    pub name: String,
    pub depends_on: Vec<String>,
    pub status: NodeStatus,
    /// Shown under the name — a duration, an elapsed timer, or nothing.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    Blocked,
    Ready,
    Dispatched,
    Success,
    Failed,
    Skipped,
    /// A definition being shown without a run behind it.
    Undefined,
}

impl NodeStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "blocked" => NodeStatus::Blocked,
            "ready" => NodeStatus::Ready,
            "dispatched" => NodeStatus::Dispatched,
            "success" => NodeStatus::Success,
            "failed" => NodeStatus::Failed,
            "skipped" => NodeStatus::Skipped,
            _ => NodeStatus::Undefined,
        }
    }

    /// Glyph in the node's box. `ascii` for non-UTF8-safe output.
    pub fn glyph(self, ascii: bool) -> &'static str {
        match (self, ascii) {
            (NodeStatus::Success, false) => "✔",
            (NodeStatus::Success, true) => "+",
            (NodeStatus::Failed, false) => "✖",
            (NodeStatus::Failed, true) => "x",
            (NodeStatus::Dispatched, false) => "◐",
            (NodeStatus::Dispatched, true) => ">",
            (NodeStatus::Ready, false) => "◔",
            (NodeStatus::Ready, true) => "-",
            (NodeStatus::Skipped, false) => "⊘",
            (NodeStatus::Skipped, true) => "/",
            (NodeStatus::Blocked, false) => "○",
            (NodeStatus::Blocked, true) => ".",
            (NodeStatus::Undefined, false) => "·",
            (NodeStatus::Undefined, true) => " ",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            NodeStatus::Success => "success",
            NodeStatus::Failed => "failed",
            NodeStatus::Dispatched => "running",
            NodeStatus::Ready => "ready",
            NodeStatus::Skipped => "skipped",
            NodeStatus::Blocked => "blocked",
            NodeStatus::Undefined => "-",
        }
    }
}

/// Rendering options.
#[derive(Debug, Clone)]
pub struct RenderOpts {
    /// Plain-ASCII box drawing and glyphs, for a non-UTF8 terminal or a pipe.
    pub ascii: bool,
    /// Terminal width. Below the laid-out width, the renderer falls back to an
    /// indented dependency list, which stays readable at any size.
    pub width: usize,
    /// Highlight this step (the TUI's selection).
    pub selected: Option<String>,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self { ascii: false, width: 100, selected: None }
    }
}

const NODE_INNER: usize = 22;
const GAP: usize = 4;

/// Longest-path column per node, plus a stable order within each column.
fn layout(nodes: &[DagNode]) -> Vec<Vec<usize>> {
    let index: BTreeMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, n)| (n.name.as_str(), i)).collect();

    // Columns via longest path. Iterating to a fixed point handles any input
    // order without needing a topological sort first.
    let mut column = vec![0usize; nodes.len()];
    for _ in 0..nodes.len() {
        let mut changed = false;
        for (i, node) in nodes.iter().enumerate() {
            let want = node
                .depends_on
                .iter()
                .filter_map(|d| index.get(d.as_str()))
                .map(|&d| column[d] + 1)
                .max()
                .unwrap_or(0);
            if want > column[i] {
                column[i] = want;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let depth = column.iter().copied().max().map_or(0, |m| m + 1);
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); depth];
    for (i, &c) in column.iter().enumerate() {
        columns[c].push(i);
    }

    // Barycenter pass: order each column by the mean row of its predecessors, so
    // a join sits next to the branches feeding it instead of across the diagram.
    let mut row_of = vec![0usize; nodes.len()];
    for col in columns.iter_mut() {
        col.sort_by(|a, b| {
            let key = |i: usize| -> (u64, &str) {
                let preds: Vec<usize> = nodes[i]
                    .depends_on
                    .iter()
                    .filter_map(|d| index.get(d.as_str()))
                    .map(|&d| row_of[d])
                    .collect();
                let bary = if preds.is_empty() {
                    u64::MAX / 2
                } else {
                    (preds.iter().sum::<usize>() * 1000 / preds.len()) as u64
                };
                (bary, nodes[i].name.as_str())
            };
            key(*a).cmp(&key(*b))
        });
        for (row, &i) in col.iter().enumerate() {
            row_of[i] = row;
        }
    }
    columns
}

/// Render the DAG. Returns display lines, no trailing newlines.
pub fn render(nodes: &[DagNode], opts: &RenderOpts) -> Vec<String> {
    if nodes.is_empty() {
        return vec!["(no steps)".to_string()];
    }
    let columns = layout(nodes);
    let laid_out_width = columns.len() * (NODE_INNER + 2) + columns.len().saturating_sub(1) * GAP;
    if laid_out_width > opts.width {
        return render_list(nodes, &columns, opts);
    }

    let (tl, tr, bl, br, h, v) = if opts.ascii {
        ("+", "+", "+", "+", "-", "|")
    } else {
        ("┌", "┐", "└", "┘", "─", "│")
    };
    let arrow = if opts.ascii { ">" } else { "▸" };

    // Every node occupies 4 lines (top, name, detail, bottom) plus a spacer.
    const NODE_LINES: usize = 4;
    const SPACER: usize = 1;
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    let height = rows * (NODE_LINES + SPACER);
    let total_width = laid_out_width;
    let mut canvas = vec![vec![' '; total_width]; height];

    let put = |canvas: &mut Vec<Vec<char>>, y: usize, x: usize, text: &str| {
        if y >= canvas.len() {
            return;
        }
        for (i, ch) in text.chars().enumerate() {
            if x + i < canvas[y].len() {
                canvas[y][x + i] = ch;
            }
        }
    };

    // Where each node's box sits, so edges can be drawn against it.
    let mut placement: BTreeMap<&str, (usize, usize)> = BTreeMap::new();

    for (col_idx, col) in columns.iter().enumerate() {
        let x = col_idx * (NODE_INNER + 2 + GAP);
        for (row_idx, &node_idx) in col.iter().enumerate() {
            let node = &nodes[node_idx];
            let y = row_idx * (NODE_LINES + SPACER);
            placement.insert(node.name.as_str(), (x, y));

            let selected = opts.selected.as_deref() == Some(node.name.as_str());
            let (tl, tr, bl, br, h) = if selected && !opts.ascii {
                ("┏", "┓", "┗", "┛", "━")
            } else {
                (tl, tr, bl, br, h)
            };

            put(&mut canvas, y, x, &format!("{tl}{}{tr}", h.repeat(NODE_INNER)));
            let title = truncate(
                &format!("{} {}", node.status.glyph(opts.ascii), node.name),
                NODE_INNER - 2,
            );
            put(&mut canvas, y + 1, x, &format!("{v} {title:<w$} {v}", w = NODE_INNER - 2));
            let detail = truncate(node.detail.as_deref().unwrap_or(""), NODE_INNER - 2);
            put(&mut canvas, y + 2, x, &format!("{v} {detail:<w$} {v}", w = NODE_INNER - 2));
            put(&mut canvas, y + 3, x, &format!("{bl}{}{br}", h.repeat(NODE_INNER)));
        }
    }

    // Edges: out of the dependency's right edge, across the gap, into the
    // dependent's left edge. Orthogonal and simple — legible beats pretty here.
    for node in nodes {
        let Some(&(to_x, to_y)) = placement.get(node.name.as_str()) else { continue };
        for dep in &node.depends_on {
            let Some(&(from_x, from_y)) = placement.get(dep.as_str()) else { continue };
            let start = from_x + NODE_INNER + 2;
            let end = to_x;
            if end <= start {
                continue;
            }
            let mid = start + (end - start) / 2;
            let src_y = from_y + 1;
            let dst_y = to_y + 1;

            for x in start..mid {
                if canvas[src_y][x] == ' ' {
                    canvas[src_y][x] = h.chars().next().unwrap();
                }
            }
            let (lo, hi) = if src_y <= dst_y { (src_y, dst_y) } else { (dst_y, src_y) };
            for y in lo..=hi {
                if canvas[y][mid] == ' ' {
                    canvas[y][mid] = v.chars().next().unwrap();
                }
            }
            for x in mid + 1..end.saturating_sub(1) {
                if canvas[dst_y][x] == ' ' {
                    canvas[dst_y][x] = h.chars().next().unwrap();
                }
            }
            if end >= 1 {
                canvas[dst_y][end - 1] = arrow.chars().next().unwrap();
            }
        }
    }

    canvas
        .into_iter()
        .map(|row| row.into_iter().collect::<String>().trim_end().to_string())
        .collect()
}

/// Narrow-terminal fallback: the same layering, as an indented list.
fn render_list(nodes: &[DagNode], columns: &[Vec<usize>], opts: &RenderOpts) -> Vec<String> {
    let mut out = Vec::new();
    for (depth, col) in columns.iter().enumerate() {
        for &i in col {
            let node = &nodes[i];
            let indent = "  ".repeat(depth);
            let mut line = format!(
                "{indent}{} {} [{}]",
                node.status.glyph(opts.ascii),
                node.name,
                node.status.label()
            );
            if let Some(detail) = &node.detail {
                line.push_str(&format!(" {detail}"));
            }
            if !node.depends_on.is_empty() {
                line.push_str(&format!(" ← {}", node.depends_on.join(", ")));
            }
            out.push(line);
        }
    }
    out
}

/// One-line legend for the status glyphs.
pub fn legend(ascii: bool) -> String {
    [
        NodeStatus::Success,
        NodeStatus::Failed,
        NodeStatus::Dispatched,
        NodeStatus::Blocked,
        NodeStatus::Skipped,
    ]
    .iter()
    .map(|s| format!("{} {}", s.glyph(ascii), s.label()))
    .collect::<Vec<_>>()
    .join("   ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    format!("{}…", s.chars().take(keep).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, deps: &[&str], status: NodeStatus) -> DagNode {
        DagNode {
            name: name.to_string(),
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            status,
            detail: None,
        }
    }

    /// extract-orders ┐
    ///                ├→ transform → {notify, archive}
    /// extract-users  ┘
    fn diamond() -> Vec<DagNode> {
        vec![
            node("extract-orders", &[], NodeStatus::Success),
            node("extract-users", &[], NodeStatus::Success),
            node("transform", &["extract-orders", "extract-users"], NodeStatus::Dispatched),
            node("notify", &["transform"], NodeStatus::Blocked),
            node("archive", &["transform"], NodeStatus::Blocked),
        ]
    }

    #[test]
    fn dependencies_sit_left_of_their_dependents() {
        let columns = layout(&diamond());
        let column_of = |name: &str| {
            columns
                .iter()
                .position(|col| col.iter().any(|&i| diamond()[i].name == name))
                .unwrap()
        };
        assert_eq!(column_of("extract-orders"), 0);
        assert_eq!(column_of("extract-users"), 0);
        assert_eq!(column_of("transform"), 1, "a join lands after every branch");
        assert_eq!(column_of("notify"), 2);
        assert_eq!(column_of("archive"), 2);
    }

    #[test]
    fn layout_is_independent_of_input_order() {
        let mut reversed = diamond();
        reversed.reverse();
        assert_eq!(layout(&diamond()).len(), layout(&reversed).len());
    }

    #[test]
    fn renders_every_step_with_its_status_glyph() {
        let lines = render(&diamond(), &RenderOpts { width: 200, ..Default::default() });
        let text = lines.join("\n");
        for name in ["extract-orders", "extract-users", "transform", "notify", "archive"] {
            assert!(text.contains(name), "missing {name} in:\n{text}");
        }
        assert!(text.contains('✔'), "a finished step shows its glyph");
        assert!(text.contains('◐'), "a running step shows its glyph");
        assert!(text.contains('▸'), "edges point at their dependents");
    }

    #[test]
    fn ascii_mode_avoids_box_drawing_and_glyphs() {
        let lines = render(
            &diamond(),
            &RenderOpts { ascii: true, width: 200, selected: None },
        );
        let text = lines.join("\n");
        assert!(text.is_ascii(), "ascii mode must stay ascii:\n{text}");
        assert!(text.contains("+---"), "boxes fall back to ASCII");
    }

    #[test]
    fn a_narrow_terminal_falls_back_to_an_indented_list() {
        let lines = render(&diamond(), &RenderOpts { width: 30, ..Default::default() });
        let text = lines.join("\n");
        assert!(!text.contains('┌'), "no boxes at this width:\n{text}");
        assert!(text.contains("← extract-orders, extract-users"), "deps are named instead");
        // Deeper steps are indented further, which is the layering made visible.
        let notify = lines.iter().find(|l| l.contains("notify")).unwrap();
        assert!(notify.starts_with("    "), "layer 2 is indented twice: {notify:?}");
    }

    #[test]
    fn selection_is_visible() {
        let lines = render(
            &diamond(),
            &RenderOpts { width: 200, selected: Some("transform".into()), ascii: false },
        );
        assert!(lines.join("\n").contains('┏'), "the selected node is drawn heavier");
    }

    #[test]
    fn long_names_and_details_are_truncated_not_wrapped() {
        let mut nodes = vec![node("a-very-long-step-name-that-will-not-fit", &[], NodeStatus::Success)];
        nodes[0].detail = Some("an equally long detail string here".into());
        let lines = render(&nodes, &RenderOpts { width: 200, ..Default::default() });
        assert!(lines.iter().all(|l| l.chars().count() <= NODE_INNER + 2));
        assert!(lines.join("\n").contains('…'));
    }

    #[test]
    fn handles_an_empty_dag() {
        assert_eq!(render(&[], &RenderOpts::default()), vec!["(no steps)".to_string()]);
    }
}
