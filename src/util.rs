use std::path::{Component, Path, PathBuf};
use std::time::Duration;

/// Canonicalize a path, falling back to the original if canonicalization fails.
pub fn canon_or_self(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Lexically normalize a path by resolving `.` and `..` components without
/// touching the filesystem.  Unlike `canonicalize()` this works even when the
/// target path does not exist yet.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components: Vec<Component> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(components.last(), Some(Component::Normal(_))) {
                    components.pop();
                } else if matches!(
                    components.last(),
                    Some(Component::RootDir) | Some(Component::Prefix(_))
                ) {
                    // Already at root, ignore the ".."
                } else {
                    components.push(component);
                }
            }
            _ => components.push(component),
        }
    }
    components.iter().collect()
}

/// Format an age in seconds as a compact relative string (e.g., "2h", "3d", "1w", "2mo").
pub fn format_compact_age(secs: u64) -> String {
    let mins = secs / 60;
    let hours = secs / 3600;
    let days = secs / 86400;
    let weeks = days / 7;
    let months = days / 30;
    let years = days / 365;

    if years > 0 {
        format!("{}y", years)
    } else if months > 0 {
        format!("{}mo", months)
    } else if weeks > 0 {
        format!("{}w", weeks)
    } else if days > 0 {
        format!("{}d", days)
    } else if hours > 0 {
        format!("{}h", hours)
    } else if mins > 0 {
        format!("{}m", mins)
    } else {
        "<1m".to_string()
    }
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

    #[test]
    fn format_compact_age_sub_minute() {
        assert_eq!(format_compact_age(0), "<1m");
        assert_eq!(format_compact_age(30), "<1m");
        assert_eq!(format_compact_age(59), "<1m");
    }

    #[test]
    fn format_compact_age_minutes() {
        assert_eq!(format_compact_age(60), "1m");
        assert_eq!(format_compact_age(300), "5m");
        assert_eq!(format_compact_age(3599), "59m");
    }

    #[test]
    fn format_compact_age_hours() {
        assert_eq!(format_compact_age(3600), "1h");
        assert_eq!(format_compact_age(7200), "2h");
        assert_eq!(format_compact_age(86399), "23h");
    }

    #[test]
    fn format_compact_age_days() {
        assert_eq!(format_compact_age(86400), "1d");
        assert_eq!(format_compact_age(259200), "3d");
        assert_eq!(format_compact_age(604799), "6d");
    }

    #[test]
    fn format_compact_age_weeks() {
        assert_eq!(format_compact_age(604800), "1w");
        assert_eq!(format_compact_age(1209600), "2w");
    }

    #[test]
    fn format_compact_age_months() {
        assert_eq!(format_compact_age(30 * 86400), "1mo");
        assert_eq!(format_compact_age(60 * 86400), "2mo");
        assert_eq!(format_compact_age(364 * 86400), "12mo");
    }

    #[test]
    fn format_compact_age_years() {
        assert_eq!(format_compact_age(365 * 86400), "1y");
        assert_eq!(format_compact_age(730 * 86400), "2y");
    }

    #[test]
    fn normalize_path_collapses_parent_dir() {
        let p = Path::new("/Users/test/repo/../wm/handle");
        assert_eq!(normalize_path(p), PathBuf::from("/Users/test/wm/handle"));
    }

    #[test]
    fn normalize_path_collapses_multiple_parent_dirs() {
        let p = Path::new("/a/b/c/../../d");
        assert_eq!(normalize_path(p), PathBuf::from("/a/d"));
    }

    #[test]
    fn normalize_path_strips_cur_dir() {
        let p = Path::new("/a/./b/./c");
        assert_eq!(normalize_path(p), PathBuf::from("/a/b/c"));
    }

    #[test]
    fn normalize_path_preserves_leading_parent() {
        let p = Path::new("../wm/handle");
        assert_eq!(normalize_path(p), PathBuf::from("../wm/handle"));
    }

    #[test]
    fn normalize_path_no_op_for_clean_path() {
        let p = Path::new("/Users/test/wm/handle");
        assert_eq!(normalize_path(p), PathBuf::from("/Users/test/wm/handle"));
    }

    #[test]
    fn normalize_path_root_parent_stays_at_root() {
        let p = Path::new("/../foo");
        assert_eq!(normalize_path(p), PathBuf::from("/foo"));
    }
}
