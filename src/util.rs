use std::path::{Path, PathBuf};
use std::time::Duration;

/// Canonicalize a path, falling back to the original if canonicalization fails.
pub fn canon_or_self(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Format a duration as a human-readable elapsed time string.
/// Used by `status` and `wait` commands.
pub fn format_elapsed_secs(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 {
            format!("{}h", h)
        } else {
            format!("{}h {}m", h, m)
        }
    }
}

/// Format a Duration as a human-readable elapsed time string (with seconds).
/// Used by `wait` command for more precise timing.
pub fn format_elapsed_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{}m {:02}s", m, s)
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{}h {:02}m", h, m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_elapsed_secs_seconds() {
        assert_eq!(format_elapsed_secs(0), "0s");
        assert_eq!(format_elapsed_secs(30), "30s");
        assert_eq!(format_elapsed_secs(59), "59s");
    }

    #[test]
    fn format_elapsed_secs_minutes() {
        assert_eq!(format_elapsed_secs(60), "1m");
        assert_eq!(format_elapsed_secs(150), "2m");
        assert_eq!(format_elapsed_secs(3599), "59m");
    }

    #[test]
    fn format_elapsed_secs_hours() {
        assert_eq!(format_elapsed_secs(3600), "1h");
        assert_eq!(format_elapsed_secs(7200), "2h");
    }

    #[test]
    fn format_elapsed_secs_hours_and_minutes() {
        assert_eq!(format_elapsed_secs(3660), "1h 1m");
        assert_eq!(format_elapsed_secs(5400), "1h 30m");
        assert_eq!(format_elapsed_secs(86400), "24h");
    }

    #[test]
    fn format_elapsed_duration_seconds() {
        assert_eq!(format_elapsed_duration(Duration::from_secs(0)), "0s");
        assert_eq!(format_elapsed_duration(Duration::from_secs(45)), "45s");
    }

    #[test]
    fn format_elapsed_duration_minutes_and_seconds() {
        assert_eq!(format_elapsed_duration(Duration::from_secs(65)), "1m 05s");
        assert_eq!(
            format_elapsed_duration(Duration::from_secs(3599)),
            "59m 59s"
        );
    }

    #[test]
    fn format_elapsed_duration_hours_and_minutes() {
        assert_eq!(format_elapsed_duration(Duration::from_secs(3600)), "1h 00m");
        assert_eq!(format_elapsed_duration(Duration::from_secs(3661)), "1h 01m");
        assert_eq!(format_elapsed_duration(Duration::from_secs(7260)), "2h 01m");
    }
}
