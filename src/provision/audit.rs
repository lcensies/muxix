use anyhow::{Context, Result};

use super::types::AuditEvent;

/// Append an audit event to the provision audit log (JSONL format).
pub fn append_audit_event(event: &AuditEvent) -> Result<()> {
    let path = crate::xdg::state_dir()?.join("provision-audit.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating state dir {}", parent.display()))?;
    }
    let mut line = serde_json::to_string(event).context("serializing audit event")?;
    line.push('\n');
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("opening audit log {}", path.display()))?;
    file.write_all(line.as_bytes())
        .context("writing audit event")?;
    Ok(())
}
