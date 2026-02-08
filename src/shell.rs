//! Shell escaping utilities.

/// Escape single quotes within a string for use inside a single-quoted shell argument.
///
/// The caller is responsible for wrapping the result in single quotes.
/// Example: `format!("'{}'", shell_escape(s))`
pub fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Quote a string for safe use as a shell argument.
///
/// Returns the string unchanged if it contains only safe characters
/// (alphanumeric, `-`, `_`, `.`, `/`). Otherwise wraps it in single quotes
/// with internal single quotes escaped. Empty strings return `''`.
pub fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
    {
        s.to_string()
    } else {
        format!("'{}'", shell_escape(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("foo bar"), "foo bar");
    }

    #[test]
    fn test_shell_escape_single_quotes() {
        assert_eq!(
            shell_escape("echo 'hello world'"),
            "echo '\\''hello world'\\''"
        );
    }

    #[test]
    fn test_shell_escape_preserves_special_chars() {
        assert_eq!(shell_escape("$HOME"), "$HOME");
        assert_eq!(shell_escape("$(cmd)"), "$(cmd)");
        assert_eq!(shell_escape("a & b"), "a & b");
    }

    #[test]
    fn test_shell_quote_safe_passthrough() {
        assert_eq!(shell_quote("hello"), "hello");
        assert_eq!(shell_quote("/usr/bin/foo"), "/usr/bin/foo");
        assert_eq!(shell_quote("my-file_v2.txt"), "my-file_v2.txt");
    }

    #[test]
    fn test_shell_quote_wraps_unsafe() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("$HOME"), "'$HOME'");
        assert_eq!(shell_quote("a & b"), "'a & b'");
    }

    #[test]
    fn test_shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_shell_quote_empty_string() {
        assert_eq!(shell_quote(""), "''");
    }
}
