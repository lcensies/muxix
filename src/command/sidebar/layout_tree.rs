//! Tmux layout tree parser, serializer, and reflow logic.
//!
//! Tmux encodes window layouts as a string like:
//!   `checksum,WxH,X,Y{child1,child2,...}`
//!
//! Where:
//! - `{children}` = horizontal split (children side by side)
//! - `[children]` = vertical split (children stacked)
//! - `WxH,X,Y,pane_id` = leaf pane
//!
//! The checksum is a 4-char hex value (tmux's rotate-right-and-add algorithm).
//!
//! This module parses the layout string into a tree, allows surgical width
//! reflow after sidebar creation, and serializes back for `select-layout`.

use tracing::debug;

use crate::cmd::Cmd;

/// A rectangle in the tmux layout coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    w: u16,
    h: u16,
    x: u16,
    y: u16,
}

/// A node in the tmux layout tree.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LayoutNode {
    Leaf {
        rect: Rect,
        pane_id: u32,
    },
    HSplit {
        rect: Rect,
        children: Vec<LayoutNode>,
    },
    VSplit {
        rect: Rect,
        children: Vec<LayoutNode>,
    },
}

impl LayoutNode {
    fn rect(&self) -> &Rect {
        match self {
            LayoutNode::Leaf { rect, .. }
            | LayoutNode::HSplit { rect, .. }
            | LayoutNode::VSplit { rect, .. } => rect,
        }
    }

    fn rect_mut(&mut self) -> &mut Rect {
        match self {
            LayoutNode::Leaf { rect, .. }
            | LayoutNode::HSplit { rect, .. }
            | LayoutNode::VSplit { rect, .. } => rect,
        }
    }

    fn width(&self) -> u16 {
        self.rect().w
    }
}

// ── Parser ──────────────────────────────────────────────────────

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn expect(&mut self, ch: u8) -> Option<()> {
        if self.peek() == Some(ch) {
            self.advance();
            Some(())
        } else {
            None
        }
    }

    fn parse_u16(&mut self) -> Option<u16> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.advance();
        }
        if self.pos == start {
            return None;
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }

    fn parse_u32(&mut self) -> Option<u32> {
        let start = self.pos;
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.advance();
        }
        if self.pos == start {
            return None;
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .ok()?
            .parse()
            .ok()
    }

    /// Parse `WxH,X,Y` prefix shared by all node types.
    fn parse_rect(&mut self) -> Option<Rect> {
        let w = self.parse_u16()?;
        self.expect(b'x')?;
        let h = self.parse_u16()?;
        self.expect(b',')?;
        let x = self.parse_u16()?;
        self.expect(b',')?;
        let y = self.parse_u16()?;
        Some(Rect { w, h, x, y })
    }

    /// Parse a single layout node (leaf or split).
    fn parse_node(&mut self) -> Option<LayoutNode> {
        let rect = self.parse_rect()?;

        match self.peek() {
            Some(b'{') => {
                self.advance();
                let children = self.parse_children(b'}')?;
                Some(LayoutNode::HSplit { rect, children })
            }
            Some(b'[') => {
                self.advance();
                let children = self.parse_children(b']')?;
                Some(LayoutNode::VSplit { rect, children })
            }
            Some(b',') => {
                self.advance();
                let pane_id = self.parse_u32()?;
                Some(LayoutNode::Leaf { rect, pane_id })
            }
            _ => None,
        }
    }

    /// Parse comma-separated children until the closing bracket.
    fn parse_children(&mut self, close: u8) -> Option<Vec<LayoutNode>> {
        let mut children = Vec::new();
        loop {
            children.push(self.parse_node()?);
            match self.peek() {
                Some(c) if c == close => {
                    self.advance();
                    return Some(children);
                }
                Some(b',') => {
                    self.advance();
                }
                _ => return None,
            }
        }
    }
}

/// Parse a tmux layout string (including checksum prefix) into a tree.
fn parse_layout(layout: &str) -> Option<LayoutNode> {
    // Skip "XXXX," checksum prefix
    let body = layout.get(5..)?;
    if layout.as_bytes().get(4).copied().is_none_or(|b| b != b',') {
        return None;
    }
    let mut parser = Parser::new(body);
    let node = parser.parse_node()?;
    // Ensure entire input was consumed
    if parser.pos == parser.input.len() {
        Some(node)
    } else {
        None
    }
}

// ── Serializer ──────────────────────────────────────────────────

/// Serialize a layout node back to tmux format (without checksum).
fn serialize_node(node: &LayoutNode) -> String {
    let mut out = String::new();
    write_node(node, &mut out);
    out
}

fn write_node(node: &LayoutNode, out: &mut String) {
    use std::fmt::Write;
    let r = node.rect();
    let _ = write!(out, "{}x{},{},{}", r.w, r.h, r.x, r.y);

    match node {
        LayoutNode::Leaf { pane_id, .. } => {
            let _ = write!(out, ",{}", pane_id);
        }
        LayoutNode::HSplit { children, .. } => {
            out.push('{');
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_node(child, out);
            }
            out.push('}');
        }
        LayoutNode::VSplit { children, .. } => {
            out.push('[');
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_node(child, out);
            }
            out.push(']');
        }
    }
}

/// Compute tmux's layout checksum (rotate-right-and-add).
fn layout_checksum(layout: &str) -> u16 {
    let mut csum: u16 = 0;
    for &b in layout.as_bytes() {
        csum = (csum >> 1) | ((csum & 1) << 15);
        csum = csum.wrapping_add(b as u16);
    }
    csum
}

/// Serialize a layout tree into a full tmux layout string with checksum.
fn serialize_layout(root: &LayoutNode) -> String {
    let body = serialize_node(root);
    let checksum = layout_checksum(&body);
    format!("{:04x},{}", checksum, body)
}

// ── Reflow ──────────────────────────────────────────────────────

/// Recursively scale a subtree's width, preserving internal proportions.
///
/// For horizontal splits, children's widths are scaled proportionally.
/// For vertical splits, all children get the parent's new width.
/// X positions are recalculated during the traversal.
fn scale_width(node: &mut LayoutNode, new_w: u16, new_x: u16) {
    let rect = node.rect_mut();
    rect.w = new_w;
    rect.x = new_x;

    match node {
        LayoutNode::HSplit { children, .. } => {
            // Children share width with 1-char separators between them.
            // parent.w = sum(child.w) + (num_children - 1)
            let seps = children.len().saturating_sub(1) as u16;
            let old_child_total: u16 = children.iter().map(|c| c.width()).sum();
            let new_avail = new_w.saturating_sub(seps);

            let mut remaining = new_avail;
            let mut cx = new_x;
            let last_idx = children.len().saturating_sub(1);

            // Collect old widths before mutating
            let old_widths: Vec<u16> = children.iter().map(|c| c.width()).collect();

            for (i, child) in children.iter_mut().enumerate() {
                let child_w = if i == last_idx {
                    // Last child gets remainder to avoid rounding gaps
                    remaining
                } else if old_child_total > 0 {
                    let scaled = ((old_widths[i] as f64) * (new_avail as f64)
                        / (old_child_total as f64))
                        .round() as u16;
                    let scaled = scaled.min(remaining);
                    remaining = remaining.saturating_sub(scaled);
                    scaled
                } else {
                    0
                };
                scale_width(child, child_w, cx);
                cx = cx.saturating_add(child_w).saturating_add(1);
            }
        }
        LayoutNode::VSplit { children, .. } => {
            // All children in a vertical split share the same width
            for child in children {
                scale_width(child, new_w, new_x);
            }
        }
        LayoutNode::Leaf { .. } => {
            // Width and x already set above
        }
    }
    // Heights and y positions are unchanged (sidebar only affects horizontal dimension)
}

/// Rebalance the window layout after a sidebar pane was added.
///
/// After `split-window -hbf`, the root is an HSplit with 2 children:
/// `{sidebar_leaf, content_tree}`. The sidebar stole width only from
/// the first pane it was split from, leaving the rest of the content
/// tree lopsided. This function scales the content subtree to fill
/// the remaining space proportionally, then applies the fixed layout
/// atomically via `select-layout`.
pub(super) fn reflow_after_sidebar_add(window_id: &str, sidebar_pane_id: &str, sidebar_width: u16) {
    let layout_str = match Cmd::new("tmux")
        .args(&["display-message", "-t", window_id, "-p", "#{window_layout}"])
        .run_and_capture_stdout()
    {
        Ok(s) => s.trim().to_string(),
        Err(_) => return,
    };

    let mut root = match parse_layout(&layout_str) {
        Some(node) => node,
        None => {
            debug!(layout = layout_str.as_str(), "failed to parse layout");
            return;
        }
    };

    // After split-window -hbf, root is HSplit with [sidebar, content_tree]
    let LayoutNode::HSplit { rect, children } = &mut root else {
        return;
    };

    if children.len() != 2 {
        debug!(
            count = children.len(),
            "expected 2 children at root after sidebar split"
        );
        return;
    }

    // Verify first child is the sidebar by pane ID
    let sidebar_num: u32 = sidebar_pane_id
        .strip_prefix('%')
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let is_sidebar =
        matches!(&children[0], LayoutNode::Leaf { pane_id, .. } if *pane_id == sidebar_num);
    if !is_sidebar {
        debug!(
            sidebar_pane_id,
            "first child is not the expected sidebar pane"
        );
        return;
    }

    // Fix sidebar to exact desired width
    children[0].rect_mut().w = sidebar_width;
    children[0].rect_mut().x = 0;

    // Scale content tree to fill remaining space
    let window_w = rect.w;
    let content_w = window_w.saturating_sub(sidebar_width).saturating_sub(1); // -1 for separator
    let content_x = sidebar_width + 1;

    scale_width(&mut children[1], content_w, content_x);

    // Apply the rebalanced layout
    let new_layout = serialize_layout(&root);
    debug!(
        window_id,
        old = layout_str.as_str(),
        new = new_layout.as_str(),
        "reflow_after_sidebar_add"
    );

    let _ = Cmd::new("tmux")
        .args(&["select-layout", "-t", window_id, &new_layout])
        .run();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_pane() {
        let layout = "1234,80x24,0,0,42";
        let node = parse_layout(layout).unwrap();
        assert_eq!(
            node,
            LayoutNode::Leaf {
                rect: Rect {
                    w: 80,
                    h: 24,
                    x: 0,
                    y: 0
                },
                pane_id: 42,
            }
        );
    }

    #[test]
    fn test_parse_hsplit() {
        // Two panes side by side: 40+39=79, +1 separator = 80
        let layout = "abcd,80x24,0,0{40x24,0,0,1,39x24,41,0,2}";
        let node = parse_layout(layout).unwrap();
        match node {
            LayoutNode::HSplit { rect, children } => {
                assert_eq!(rect.w, 80);
                assert_eq!(children.len(), 2);
                assert_eq!(children[0].width(), 40);
                assert_eq!(children[1].width(), 39);
            }
            _ => panic!("expected HSplit"),
        }
    }

    #[test]
    fn test_parse_vsplit() {
        // Two panes stacked: 12+11=23, +1 separator = 24
        let layout = "abcd,80x24,0,0[80x12,0,0,1,80x11,0,13,2]";
        let node = parse_layout(layout).unwrap();
        match node {
            LayoutNode::VSplit { rect, children } => {
                assert_eq!(rect.h, 24);
                assert_eq!(children.len(), 2);
                assert_eq!(children[0].rect().h, 12);
                assert_eq!(children[1].rect().h, 11);
            }
            _ => panic!("expected VSplit"),
        }
    }

    #[test]
    fn test_parse_nested() {
        // Real layout from tmux: HSplit containing [VSplit, Leaf]
        let layout = "123a,186x44,0,0{93x44,0,0[93x22,0,0,1189,93x21,0,23,1394],92x44,94,0,1387}";
        let node = parse_layout(layout).unwrap();
        match node {
            LayoutNode::HSplit { children, .. } => {
                assert_eq!(children.len(), 2);
                // First child is a VSplit
                match &children[0] {
                    LayoutNode::VSplit { children: vc, .. } => {
                        assert_eq!(vc.len(), 2);
                    }
                    _ => panic!("expected VSplit as first child"),
                }
                // Second child is a Leaf
                assert!(matches!(
                    &children[1],
                    LayoutNode::Leaf { pane_id: 1387, .. }
                ));
            }
            _ => panic!("expected HSplit"),
        }
    }

    /// Parse and serialize every real layout, verify exact roundtrip.
    #[test]
    fn test_roundtrip_real_layouts() {
        // Real layouts captured from a live tmux session
        let layouts = [
            // HSplit with nested VSplit
            "9d0a,373x79,0,0{205x79,0,0[205x39,0,0,1070,205x39,0,40,1073],167x79,206,0,1072}",
            // Simple HSplit (2 panes)
            "6804,373x79,0,0{205x79,0,0,1075,167x79,206,0,1077}",
            "1bd3,373x79,0,0{242x79,0,0,510,130x79,243,0,532}",
            "f6ce,373x79,0,0{211x79,0,0,509,161x79,212,0,986}",
            "7e05,373x79,0,0{205x79,0,0,988,167x79,206,0,989}",
            "37ce,373x79,0,0{221x79,0,0,521,151x79,222,0,528}",
            // HSplit where second child is a VSplit
            "c6bc,373x79,0,0{212x79,0,0,634,160x79,213,0[160x20,213,0,636,160x58,213,21,637]}",
            // Single pane
            "f64a,373x79,0,0,640",
            "f652,373x79,0,0,648",
            "f651,373x79,0,0,666",
            // Smaller terminal layouts
            "ec38,373x79,0,0,67",
            // Complex: HSplit with two nested VSplits
            "0e04,186x44,0,0{100x44,0,0[100x21,0,0,980,100x22,0,22,817],85x44,101,0[85x21,101,0,985,85x22,101,22,1169]}",
            // HSplit with leaf + nested VSplit
            "f910,186x44,0,0{91x44,0,0,350,94x44,92,0[94x21,92,0,512,94x22,92,22,1074]}",
            "123a,186x44,0,0{93x44,0,0[93x22,0,0,1189,93x21,0,23,1394],92x44,94,0,1387}",
            // 3 children at root (sidebar + 2 content panes)
            "7fb5,186x44,0,0{25x44,0,0,1395,91x44,26,0,1196,68x44,118,0,1307}",
            "184f,186x44,0,0,1396",
        ];

        for layout in layouts {
            let node = parse_layout(layout).expect(&format!("failed to parse: {}", layout));
            let result = serialize_layout(&node);
            assert_eq!(result, layout, "roundtrip failed");
        }
    }

    #[test]
    fn test_checksum_known_values() {
        // Verify checksum against multiple real tmux layouts
        let cases = [
            (
                "373x79,0,0{205x79,0,0[205x39,0,0,1070,205x39,0,40,1073],167x79,206,0,1072}",
                "9d0a",
            ),
            ("373x79,0,0{205x79,0,0,1075,167x79,206,0,1077}", "6804"),
            ("373x79,0,0,640", "f64a"),
            (
                "186x44,0,0{93x44,0,0[93x22,0,0,1189,93x21,0,23,1394],92x44,94,0,1387}",
                "123a",
            ),
        ];
        for (body, expected_hex) in cases {
            let expected = u16::from_str_radix(expected_hex, 16).unwrap();
            assert_eq!(
                layout_checksum(body),
                expected,
                "checksum mismatch for: {}",
                body
            );
        }
    }

    #[test]
    fn test_scale_width_leaf() {
        let mut node = LayoutNode::Leaf {
            rect: Rect {
                w: 100,
                h: 50,
                x: 0,
                y: 0,
            },
            pane_id: 1,
        };
        scale_width(&mut node, 80, 10);
        assert_eq!(node.rect().w, 80);
        assert_eq!(node.rect().x, 10);
        assert_eq!(node.rect().h, 50); // height unchanged
    }

    #[test]
    fn test_scale_width_hsplit_proportional() {
        // HSplit 200 wide: children 100+99=199, 1 separator
        let mut node = LayoutNode::HSplit {
            rect: Rect {
                w: 200,
                h: 50,
                x: 0,
                y: 0,
            },
            children: vec![
                LayoutNode::Leaf {
                    rect: Rect {
                        w: 100,
                        h: 50,
                        x: 0,
                        y: 0,
                    },
                    pane_id: 1,
                },
                LayoutNode::Leaf {
                    rect: Rect {
                        w: 99,
                        h: 50,
                        x: 101,
                        y: 0,
                    },
                    pane_id: 2,
                },
            ],
        };

        // Scale to 150 wide, starting at x=30
        scale_width(&mut node, 150, 30);

        assert_eq!(node.rect().w, 150);
        assert_eq!(node.rect().x, 30);
        // Available for children: 150 - 1 separator = 149
        // Old total: 199. Scale factor = 149/199
        // Child 1: round(100 * 149/199) = round(74.87) = 75
        // Child 2: 149 - 75 = 74
        let children = match &node {
            LayoutNode::HSplit { children, .. } => children,
            _ => panic!(),
        };
        assert_eq!(children[0].width(), 75);
        assert_eq!(children[1].width(), 74);
        // x positions: child 0 at 30, child 1 at 30+75+1=106
        assert_eq!(children[0].rect().x, 30);
        assert_eq!(children[1].rect().x, 106);
    }

    #[test]
    fn test_scale_width_vsplit() {
        let mut node = LayoutNode::VSplit {
            rect: Rect {
                w: 100,
                h: 50,
                x: 0,
                y: 0,
            },
            children: vec![
                LayoutNode::Leaf {
                    rect: Rect {
                        w: 100,
                        h: 24,
                        x: 0,
                        y: 0,
                    },
                    pane_id: 1,
                },
                LayoutNode::Leaf {
                    rect: Rect {
                        w: 100,
                        h: 25,
                        x: 0,
                        y: 25,
                    },
                    pane_id: 2,
                },
            ],
        };

        scale_width(&mut node, 80, 20);

        // Both children should get the same new width
        let children = match &node {
            LayoutNode::VSplit { children, .. } => children,
            _ => panic!(),
        };
        assert_eq!(children[0].width(), 80);
        assert_eq!(children[1].width(), 80);
        assert_eq!(children[0].rect().x, 20);
        assert_eq!(children[1].rect().x, 20);
        // Heights unchanged
        assert_eq!(children[0].rect().h, 24);
        assert_eq!(children[1].rect().h, 25);
    }

    /// Simulate what reflow_after_sidebar_add does: sidebar + content VSplit.
    #[test]
    fn test_scale_sidebar_plus_vsplit_content() {
        // Layout after split-window -hbf: {sidebar(35), content_vsplit(150)}
        // Window is 186 wide: 35 + 1 separator + 150 = 186
        let layout =
            "0000,186x44,0,0{35x44,0,0,999,150x44,36,0[150x22,36,0,1189,150x21,36,23,1394]}";
        let mut root = parse_layout(layout).unwrap();

        if let LayoutNode::HSplit { children, .. } = &mut root {
            // Scale content to fill remaining: 186 - 35 - 1 = 150 (already correct here)
            let content_w = 186u16.saturating_sub(35).saturating_sub(1);
            scale_width(&mut children[1], content_w, 36);

            assert_eq!(children[1].width(), content_w);
            // All VSplit children should have the same width
            if let LayoutNode::VSplit { children: vc, .. } = &children[1] {
                assert_eq!(vc[0].width(), content_w);
                assert_eq!(vc[1].width(), content_w);
            }
        }
    }

    /// Test reflow on a real layout: HSplit with two nested VSplits.
    /// Simulates sidebar insertion stealing width from the left VSplit only.
    #[test]
    fn test_reflow_real_complex_layout() {
        // Original: 186x44 {100x44[2 panes], 85x44[2 panes]}
        // After sidebar(25) added via split-window -hbf on first pane:
        // Root becomes: {25x44(sidebar), 160x44{75x44[2 panes], 85x44[2 panes]}}
        // The left VSplit shrunk from 100 to 75, but right stayed at 85. Lopsided!
        //
        // After reflow: content tree gets 186-25-1=160 wide.
        // Old content was {75 + 85} = 160 (already fits, but was already in this case).
        // In a real scenario where old content is squeezed, scale preserves ratios.

        // Simulate a case where sidebar stole more from left:
        // Window=200, sidebar=30, content was originally {90, 79} (total 169+1sep=170)
        // After split-window: {30(sidebar), 169{60, 79}} - left shrunk from 90 to 60
        // Reflow should give content 200-30-1=169, distribute: 60+79=139 old -> 168 new
        // Scale: 60*(168/139)=72.5->73, 79*(168/139)=95.6->95. Total=168. With sep: 169.
        let layout = "0000,200x50,0,0{30x50,0,0,100,169x50,31,0{60x50,31,0,101,79x50,92,0,102}}";
        let mut root = parse_layout(layout).unwrap();

        if let LayoutNode::HSplit { children, .. } = &mut root {
            assert_eq!(children.len(), 2);

            // Reflow content
            let content_w = 200u16 - 30 - 1;
            scale_width(&mut children[1], content_w, 31);

            assert_eq!(children[1].width(), content_w); // 169
            if let LayoutNode::HSplit {
                children: content, ..
            } = &children[1]
            {
                // Children should sum to 169-1=168 (minus 1 separator)
                let sum: u16 = content.iter().map(|c| c.width()).sum();
                assert_eq!(sum, 168);
                // Proportions should be roughly preserved (60:79 -> ~73:95)
                assert!(content[0].width() > 70 && content[0].width() < 76);
                assert!(content[1].width() > 92 && content[1].width() < 98);
            }
        }
    }
}
