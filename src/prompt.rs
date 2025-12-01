use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Prompt {
    Inline(String),
    FromFile(PathBuf),
}

impl Prompt {
    /// Read the prompt content as a string.
    /// For Inline prompts, returns the string directly.
    /// For FromFile prompts, reads the file content.
    pub fn read_content(&self) -> Result<String> {
        match self {
            Prompt::Inline(s) => Ok(s.clone()),
            Prompt::FromFile(path) => fs::read_to_string(path)
                .with_context(|| format!("Failed to read prompt file: {}", path.display())),
        }
    }
}

#[derive(Debug, Deserialize, Default)]
pub struct PromptMetadata {
    #[serde(default)]
    pub foreach: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug)]
pub struct PromptDocument {
    pub body: String,
    pub meta: PromptMetadata,
}

/// Split frontmatter from markdown content.
/// Returns (Some(frontmatter_yaml), body) if frontmatter exists, or (None, content) if not.
fn split_frontmatter(content: &str) -> (Option<String>, &str) {
    let lines: Vec<&str> = content.lines().collect();

    // Check if content starts with "---"
    if lines.is_empty() || lines[0].trim() != "---" {
        return (None, content);
    }

    // Find the closing "---" or "..."
    let closing_idx = lines.iter().skip(1).position(|line| {
        let trimmed = line.trim();
        trimmed == "---" || trimmed == "..."
    });

    match closing_idx {
        Some(idx) => {
            // closing_idx is relative to skip(1), so actual index is idx + 1
            let actual_idx = idx + 1;
            let frontmatter = lines[1..actual_idx].join("\n");
            // Body starts after the closing fence
            let body_start = lines
                .iter()
                .take(actual_idx + 1)
                .map(|l| l.len() + 1)
                .sum::<usize>();
            let body = &content[body_start.min(content.len())..];
            (Some(frontmatter), body)
        }
        None => {
            // No closing fence found, treat entire content as body
            (None, content)
        }
    }
}

/// Parse a prompt document, extracting frontmatter metadata and body.
pub fn parse_prompt_document(prompt: &Prompt) -> Result<PromptDocument> {
    // Store the file content to avoid dangling reference
    let content_storage: String;
    let content = match prompt {
        Prompt::Inline(text) => text.as_str(),
        Prompt::FromFile(path) => {
            content_storage = fs::read_to_string(path)
                .with_context(|| format!("Failed to read prompt file: {}", path.display()))?;
            &content_storage
        }
    };

    let (frontmatter_yaml, body) = split_frontmatter(content);

    let meta = if let Some(ref yaml) = frontmatter_yaml {
        serde_yaml::from_str(yaml).context("Failed to parse YAML frontmatter")?
    } else {
        PromptMetadata::default()
    };

    Ok(PromptDocument {
        body: body.to_string(),
        meta,
    })
}

/// Convert frontmatter foreach (BTreeMap<String, Vec<String>>) to matrix rows.
/// Validates that all value lists have equal length (zip constraint).
pub fn foreach_from_frontmatter(
    foreach_map: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<BTreeMap<String, String>>> {
    if foreach_map.is_empty() {
        return Err(anyhow::anyhow!(
            "foreach in frontmatter must include at least one variable"
        ));
    }

    // Get the first list length as reference
    let expected_len = foreach_map.values().next().unwrap().len();

    if expected_len == 0 {
        return Err(anyhow::anyhow!(
            "foreach variables must have at least one value"
        ));
    }

    // Validate all lists have the same length
    for (key, values) in foreach_map.iter() {
        if values.len() != expected_len {
            return Err(anyhow::anyhow!(
                "All foreach variables must have the same number of values (expected {}, but '{}' has {})",
                expected_len,
                key,
                values.len()
            ));
        }
    }

    // Zip values by index to create row dictionaries
    let mut rows = Vec::with_capacity(expected_len);
    for idx in 0..expected_len {
        let mut row = BTreeMap::new();
        for (key, values) in foreach_map {
            row.insert(key.clone(), values[idx].clone());
        }
        rows.push(row);
    }

    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn split_frontmatter_extracts_yaml_and_body() {
        let content = "---\nkey: value\n---\n\nBody content here";
        let (frontmatter, body) = split_frontmatter(content);

        assert_eq!(frontmatter, Some("key: value".to_string()));
        assert_eq!(body, "\nBody content here");
    }

    #[test]
    fn split_frontmatter_handles_no_frontmatter() {
        let content = "Just body content";
        let (frontmatter, body) = split_frontmatter(content);

        assert_eq!(frontmatter, None);
        assert_eq!(body, "Just body content");
    }

    #[test]
    fn split_frontmatter_handles_missing_closing_fence() {
        let content = "---\nkey: value\nno closing fence";
        let (frontmatter, body) = split_frontmatter(content);

        assert_eq!(frontmatter, None);
        assert_eq!(body, "---\nkey: value\nno closing fence");
    }

    #[test]
    fn parse_prompt_document_with_frontmatter() {
        let content = "---\nforeach:\n  platform: [iOS, Android]\n---\n\nBuild for {{ platform }}";
        let prompt = Prompt::Inline(content.to_string());
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "\nBuild for {{ platform }}");
        assert!(doc.meta.foreach.is_some());

        let foreach = doc.meta.foreach.unwrap();
        assert_eq!(
            foreach.get("platform").unwrap(),
            &vec!["iOS".to_string(), "Android".to_string()]
        );
    }

    #[test]
    fn parse_prompt_document_without_frontmatter() {
        let content = "Build for {{ platform }}";
        let prompt = Prompt::Inline(content.to_string());
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "Build for {{ platform }}");
        assert!(doc.meta.foreach.is_none());
    }

    #[test]
    fn parse_prompt_document_from_file_with_frontmatter() {
        let content = "---\nforeach:\n  platform: [iOS, Android]\n  lang: [swift, kotlin]\n---\n\nBuild for {{ platform }} using {{ lang }}";
        let mut temp_file = NamedTempFile::new().expect("create temp file");
        write!(temp_file, "{}", content).expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "\nBuild for {{ platform }} using {{ lang }}");
        assert!(doc.meta.foreach.is_some());

        let foreach = doc.meta.foreach.unwrap();
        assert_eq!(foreach.len(), 2);
        assert_eq!(
            foreach.get("platform").unwrap(),
            &vec!["iOS".to_string(), "Android".to_string()]
        );
        assert_eq!(
            foreach.get("lang").unwrap(),
            &vec!["swift".to_string(), "kotlin".to_string()]
        );
    }

    #[test]
    fn parse_prompt_document_from_file_without_frontmatter() {
        let content = "Build for {{ platform }}";
        let mut temp_file = NamedTempFile::new().expect("create temp file");
        write!(temp_file, "{}", content).expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "Build for {{ platform }}");
        assert!(doc.meta.foreach.is_none());
    }

    #[test]
    fn foreach_from_frontmatter_creates_rows() {
        let mut map = BTreeMap::new();
        map.insert(
            "platform".to_string(),
            vec!["iOS".to_string(), "Android".to_string()],
        );
        map.insert(
            "lang".to_string(),
            vec!["swift".to_string(), "kotlin".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("platform").unwrap(), "iOS");
        assert_eq!(rows[0].get("lang").unwrap(), "swift");
        assert_eq!(rows[1].get("platform").unwrap(), "Android");
        assert_eq!(rows[1].get("lang").unwrap(), "kotlin");
    }

    #[test]
    fn foreach_from_frontmatter_requires_equal_lengths() {
        let mut map = BTreeMap::new();
        map.insert(
            "platform".to_string(),
            vec!["iOS".to_string(), "Android".to_string()],
        );
        map.insert("lang".to_string(), vec!["swift".to_string()]);

        let result = foreach_from_frontmatter(&map);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("same number of values")
        );
    }

    #[test]
    fn foreach_from_frontmatter_rejects_empty_values() {
        let mut map = BTreeMap::new();
        map.insert("platform".to_string(), vec![]);

        let result = foreach_from_frontmatter(&map);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at least one value")
        );
    }
}
