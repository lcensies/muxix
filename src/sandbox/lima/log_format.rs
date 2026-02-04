//! Parse and reformat limactl logrus-style log lines for clean display.

use console::strip_ansi_codes;
use regex::Regex;
use std::sync::LazyLock;

/// Regex to parse logrus text format: time="..." level=... msg="..."
/// Captures: 1=level, 2=msg (with escaped quotes inside)
static LOGRUS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^time="[^"]*"\s+level=(\w+)\s+msg="((?:[^"\\]|\\.)*)""#).unwrap()
});

/// Info-level messages containing these substrings are filtered out entirely.
/// Warning/error messages are never filtered.
const FILTERED_SUBSTRINGS: &[&str] = &["Terminal is not available", "Not forwarding TCP"];

/// Clean a non-logrus line by stripping ANSI escapes and handling carriage returns.
///
/// Installers (e.g. Claude Code) use `\r` to overwrite lines for progress display.
/// We take the last `\r`-delimited segment to get the final state of the line.
fn clean_raw_line(line: &str) -> Option<String> {
    // Take the last \r-delimited segment (the final "frame" of a progress update)
    let last_segment = line.rsplit('\r').next().unwrap_or(line);
    let stripped = strip_ansi_codes(last_segment);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!("  {}", trimmed))
}

/// Format a limactl logrus log line into a clean display string.
///
/// Returns `None` if the line should be filtered out.
/// Returns `Some(formatted)` with the clean message.
/// Lines that don't match logrus format are cleaned of ANSI escapes and passed through.
pub fn format_lima_log_line(line: &str) -> Option<String> {
    let Some(caps) = LOGRUS_RE.captures(line) else {
        // Not a logrus line -- strip ANSI escapes and \r artifacts
        return clean_raw_line(line);
    };

    let level = &caps[1];
    let msg = caps[2].replace("\\\"", "\"");

    // Only filter noisy messages at low severity levels
    let is_low_severity = matches!(level, "info" | "debug" | "trace");
    if is_low_severity && FILTERED_SUBSTRINGS.iter().any(|p| msg.contains(p)) {
        return None;
    }

    let prefix = match level {
        "warning" | "warn" => "[WARN] ",
        "error" => "[ERROR] ",
        _ => "",
    };

    Some(format!("  {}{}", prefix, msg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_info_message() {
        let line = r#"time="2026-02-06T07:30:37+02:00" level=info msg="Starting the instance \"wm-415bdd35\" with internal VM driver \"vz\"""#;
        assert_eq!(
            format_lima_log_line(line),
            Some(
                r#"  Starting the instance "wm-415bdd35" with internal VM driver "vz""#.to_string()
            )
        );
    }

    #[test]
    fn test_terminal_not_available_filtered() {
        let line = r#"time="2026-02-06T07:30:37+02:00" level=info msg="Terminal is not available, proceeding without opening an editor""#;
        assert_eq!(format_lima_log_line(line), None);
    }

    #[test]
    fn test_tcp_forwarding_filtered() {
        let line =
            r#"time="2026-02-06T07:30:37+02:00" level=info msg="Not forwarding TCP 127.0.0.53:53""#;
        assert_eq!(format_lima_log_line(line), None);
    }

    #[test]
    fn test_warning_level() {
        let line = r#"time="2026-02-06T07:30:37+02:00" level=warning msg="something went wrong""#;
        assert_eq!(
            format_lima_log_line(line),
            Some("  [WARN] something went wrong".to_string())
        );
    }

    #[test]
    fn test_error_level() {
        let line = r#"time="2026-02-06T07:30:37+02:00" level=error msg="fatal error occurred""#;
        assert_eq!(
            format_lima_log_line(line),
            Some("  [ERROR] fatal error occurred".to_string())
        );
    }

    #[test]
    fn test_warning_with_noisy_substring_not_filtered() {
        // Warnings should never be filtered, even if they match a noisy substring
        let line = r#"time="2026-02-06T07:30:37+02:00" level=warning msg="Not forwarding TCP due to critical issue""#;
        assert_eq!(
            format_lima_log_line(line),
            Some("  [WARN] Not forwarding TCP due to critical issue".to_string())
        );
    }

    #[test]
    fn test_hostagent_prefix_preserved() {
        let line = r#"time="2026-02-06T07:30:37+02:00" level=info msg="[hostagent] Waiting for the essential requirement 1 of 5: \"ssh\"""#;
        let result = format_lima_log_line(line).unwrap();
        assert!(result.contains("[hostagent]"));
        assert!(result.contains("Waiting for the essential requirement"));
    }

    #[test]
    fn test_non_logrus_line_passthrough() {
        let line = "some random output line";
        assert_eq!(
            format_lima_log_line(line),
            Some("  some random output line".to_string())
        );
    }

    #[test]
    fn test_ansi_escape_stripping() {
        let line = "\x1b[?2026l\x1b[?2026h\r\x1b[1AInstalling Cl\x1b[1Cude C\x1b[1Cde n\x1b[2Cive build\x1b[1Clatest...";
        let result = format_lima_log_line(line).unwrap();
        assert!(!result.contains('\x1b'));
        assert!(result.contains("Installing"));
    }

    #[test]
    fn test_carriage_return_takes_last_segment() {
        // Simulates progress bar: first segment is old, last is current
        let line = "old progress 50%\rnew progress 100%";
        assert_eq!(
            format_lima_log_line(line),
            Some("  new progress 100%".to_string())
        );
    }

    #[test]
    fn test_blank_after_stripping_filtered() {
        let line = "\x1b[?2026h\r\x1b[1A";
        assert_eq!(format_lima_log_line(line), None);
    }

    #[test]
    fn test_extra_key_values_ignored() {
        let line = r#"time="2026-02-06T07:30:37+02:00" level=info msg="Attempting to download the image" arch=aarch64 digest= location="https://example.com/image.qcow2""#;
        assert_eq!(
            format_lima_log_line(line),
            Some("  Attempting to download the image".to_string())
        );
    }
}
