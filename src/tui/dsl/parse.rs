//! YAML front-end for the layout DSL.

use super::node::Node;

/// Parse a YAML document into a [`Node`] tree.
pub fn parse_yaml(input: &str) -> Result<Node, String> {
    serde_yaml::from_str::<Node>(input).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::dsl::node::Size;

    #[test]
    fn parses_nested_tree() {
        let node = parse_yaml(
            "type: col\nchildren:\n  - { type: text, content: \"hi\", size: { length: 2 } }\n  - { type: spacer }\n",
        )
        .unwrap();
        match node {
            Node::Col { children, .. } => {
                assert_eq!(children.len(), 2);
                assert!(matches!(children[0], Node::Text { size: Size::Length(2), .. }));
                assert!(matches!(children[1], Node::Spacer { .. }));
            }
            other => panic!("expected col, got {other:?}"),
        }
    }

    #[test]
    fn default_size_is_fill() {
        let node = parse_yaml("type: text\ncontent: x\n").unwrap();
        // Text defaults to a single line.
        assert_eq!(node.size(), Size::Length(1));
    }

    #[test]
    fn unknown_type_errors() {
        assert!(parse_yaml("type: bogus\n").is_err());
    }

    #[test]
    fn missing_required_field_errors() {
        // `text` requires `content`.
        assert!(parse_yaml("type: text\n").is_err());
    }
}
