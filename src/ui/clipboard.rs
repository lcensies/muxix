//! Read plain text from the system clipboard.

use std::process::Command;

/// Read plain text from the system clipboard.
/// Returns `None` if the clipboard is empty or no clipboard tool is available.
pub fn read_text() -> Option<String> {
    read_text_platform()
}

#[cfg(target_os = "macos")]
fn read_text_platform() -> Option<String> {
    let out = Command::new("pbpaste").output().ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).into_owned();
    if s.is_empty() { None } else { Some(s) }
}

#[cfg(target_os = "linux")]
fn read_text_platform() -> Option<String> {
    // Try wl-paste first (Wayland), then xclip (X11)
    if let Ok(out) = Command::new("wl-paste").arg("-n").output()
        && out.status.success()
        && !out.stdout.is_empty()
    {
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        if !s.is_empty() {
            return Some(s);
        }
    }

    if let Ok(out) = Command::new("xclip")
        .args(["-selection", "clipboard", "-o"])
        .output()
        && out.status.success()
        && !out.stdout.is_empty()
    {
        let s = String::from_utf8_lossy(&out.stdout).into_owned();
        if !s.is_empty() {
            return Some(s);
        }
    }

    None
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn read_text_platform() -> Option<String> {
    None
}
