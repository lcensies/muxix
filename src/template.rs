use anyhow::{Context, Result, anyhow};
use minijinja::{AutoEscape, Environment};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

/// Reserved template variable names that cannot be used in foreach
const RESERVED_TEMPLATE_KEYS: &[&str] = &["base_name", "agent", "num", "foreach_vars"];

#[derive(Debug, Clone)]
pub struct WorktreeSpec {
    pub branch_name: String,
    pub agent: Option<String>,
    pub template_context: JsonValue,
}

pub type TemplateEnv = Environment<'static>;

/// Create and configure the template environment with filters and escape settings.
pub fn create_template_env() -> TemplateEnv {
    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_keep_trailing_newline(true);
    env.add_filter("slugify", slugify_filter);
    env
}

/// Render a prompt body string with the given template context.
pub fn render_prompt_body(body: &str, env: &TemplateEnv, context: &JsonValue) -> Result<String> {
    env.render_str(body, context)
        .context("Failed to render prompt template")
}

pub fn generate_worktree_specs(
    base_name: &str,
    agents: &[String],
    count: Option<u32>,
    foreach_rows: Option<&[BTreeMap<String, String>]>,
    env: &TemplateEnv,
    branch_template: &str,
) -> Result<Vec<WorktreeSpec>> {
    let is_multi_mode = foreach_rows.is_some() || count.is_some() || agents.len() > 1;

    if !is_multi_mode {
        let agent = agents.first().cloned();
        let num: Option<u32> = None;
        let foreach_vars = BTreeMap::<String, String>::new();
        let context = build_template_context(base_name, &agent, &num, &foreach_vars);

        // Intentional: in single-agent/instance mode the CLI keeps the provided
        // branch name verbatim so users can opt into templating only when they
        // request multiple worktrees.
        return Ok(vec![WorktreeSpec {
            branch_name: base_name.to_string(),
            agent,
            template_context: context,
        }]);
    }

    if let Some(rows) = foreach_rows {
        return rows
            .iter()
            .map(|vars| build_spec(env, branch_template, base_name, None, None, vars.clone()))
            .collect();
    }

    if let Some(times) = count {
        let iterations = times as usize;
        let default_agent = agents.first().cloned();
        let mut specs = Vec::with_capacity(iterations);
        for idx in 0..iterations {
            let num = Some((idx + 1) as u32);
            specs.push(build_spec(
                env,
                branch_template,
                base_name,
                default_agent.clone(),
                num,
                BTreeMap::new(),
            )?);
        }
        return Ok(specs);
    }

    if agents.is_empty() {
        return Ok(vec![build_spec(
            env,
            branch_template,
            base_name,
            None,
            None,
            BTreeMap::new(),
        )?]);
    }

    let mut specs = Vec::with_capacity(agents.len());
    for agent_name in agents {
        specs.push(build_spec(
            env,
            branch_template,
            base_name,
            Some(agent_name.clone()),
            None,
            BTreeMap::new(),
        )?);
    }
    Ok(specs)
}

fn build_spec(
    env: &TemplateEnv,
    branch_template: &str,
    base_name: &str,
    agent: Option<String>,
    num: Option<u32>,
    foreach_vars: BTreeMap<String, String>,
) -> Result<WorktreeSpec> {
    // Extract agent from foreach_vars if present (treat "agent" as a special reserved key)
    let effective_agent = agent.or_else(|| foreach_vars.get("agent").cloned());

    let context = build_template_context(base_name, &effective_agent, &num, &foreach_vars);
    let branch_name = env
        .render_str(branch_template, &context)
        .context("Failed to render branch template")?;
    Ok(WorktreeSpec {
        branch_name,
        agent: effective_agent,
        template_context: context,
    })
}

/// Extract the display name from an agent path or name.
/// For paths like "/usr/local/bin/claude", returns "claude".
/// For simple names like "claude", returns as-is.
fn agent_display_name(agent: &str) -> String {
    Path::new(agent)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(agent)
        .to_string()
}

fn build_template_context(
    base_name: &str,
    agent: &Option<String>,
    num: &Option<u32>,
    foreach_vars: &BTreeMap<String, String>,
) -> JsonValue {
    let mut context = JsonMap::new();
    context.insert(
        "base_name".to_string(),
        JsonValue::String(base_name.to_string()),
    );

    // Use just the filename (without path) for template context so branch names
    // are clean (e.g., "feature-claude" not "feature-usr-local-bin-claude")
    let agent_value = agent
        .as_ref()
        .map(|value| JsonValue::String(agent_display_name(value)))
        .unwrap_or(JsonValue::Null);
    context.insert("agent".to_string(), agent_value);

    let num_value = num
        .as_ref()
        .map(|value| JsonValue::Number(JsonNumber::from(*value)))
        .unwrap_or(JsonValue::Null);
    context.insert("num".to_string(), num_value);

    let mut foreach_json = JsonMap::new();
    for (key, value) in foreach_vars {
        // Filter out ALL reserved keys to avoid collisions in templates
        // Reserved keys: base_name, agent, num, foreach_vars
        if !RESERVED_TEMPLATE_KEYS.contains(&key.as_str()) {
            foreach_json.insert(key.clone(), JsonValue::String(value.clone()));
            context.insert(key.clone(), JsonValue::String(value.clone()));
        }
    }
    context.insert("foreach_vars".to_string(), JsonValue::Object(foreach_json));

    JsonValue::Object(context)
}

pub fn parse_foreach_matrix(input: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let mut columns: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen = HashSet::new();

    for raw in input.split(';') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (key, values_str) = trimmed.split_once(':').ok_or_else(|| {
            anyhow!(
                "Invalid --foreach segment '{}'. Use the format name:value1,value2",
                trimmed
            )
        })?;

        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!(
                "Invalid --foreach segment '{}': variable name cannot be empty",
                trimmed
            ));
        }
        if !seen.insert(key.to_string()) {
            return Err(anyhow!(
                "Duplicate variable '{}' found in --foreach option",
                key
            ));
        }

        let values: Vec<String> = values_str
            .split(',')
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .collect();

        if values.is_empty() {
            return Err(anyhow!(
                "Variable '{}' must have at least one value in --foreach",
                key
            ));
        }

        columns.push((key.to_string(), values));
    }

    if columns.is_empty() {
        return Err(anyhow!(
            "--foreach must include at least one variable with values"
        ));
    }

    let expected_len = columns[0].1.len();
    if columns
        .iter()
        .any(|(_, values)| values.len() != expected_len)
    {
        return Err(anyhow!(
            "All --foreach variables must have the same number of values"
        ));
    }

    let mut rows = Vec::with_capacity(expected_len);
    for idx in 0..expected_len {
        let mut map = BTreeMap::new();
        for (key, values) in &columns {
            map.insert(key.clone(), values[idx].clone());
        }
        rows.push(map);
    }

    Ok(rows)
}

fn slugify_filter(input: String) -> String {
    input
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' => c,
            _ => '-',
        })
        .collect::<String>()
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::Prompt;
    use std::fs;
    use std::path::PathBuf;

    fn create_test_env() -> TemplateEnv {
        create_template_env()
    }

    #[test]
    fn parse_foreach_matrix_parses_rows() {
        let rows = parse_foreach_matrix("env:dev,prod;region:us,eu").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("env").unwrap(), "dev");
        assert_eq!(rows[0].get("region").unwrap(), "us");
        assert_eq!(rows[1].get("env").unwrap(), "prod");
        assert_eq!(rows[1].get("region").unwrap(), "eu");
    }

    #[test]
    fn parse_foreach_matrix_requires_matching_lengths() {
        assert!(parse_foreach_matrix("env:dev,prod;region:us").is_err());
    }

    #[test]
    fn generate_specs_with_agents() {
        let env = create_test_env();
        let agents = vec!["claude".to_string(), "gemini".to_string()];
        let specs = generate_worktree_specs(
            "feature",
            &agents,
            None,
            None,
            &env,
            "{{ base_name }}{% if agent %}-{{ agent }}{% endif %}",
        )
        .expect("specs");
        let summary: Vec<(String, Option<String>)> = specs
            .into_iter()
            .map(|spec| (spec.branch_name, spec.agent))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("feature-claude".to_string(), Some("claude".to_string())),
                ("feature-gemini".to_string(), Some("gemini".to_string()))
            ]
        );
    }

    #[test]
    fn generate_specs_with_count_assigns_numbers() {
        let env = create_test_env();
        let specs = generate_worktree_specs(
            "feature",
            &[],
            Some(2),
            None,
            &env,
            "{{ base_name }}{% if num %}-{{ num }}{% endif %}",
        )
        .expect("specs");
        let names: Vec<String> = specs.into_iter().map(|s| s.branch_name).collect();
        assert_eq!(
            names,
            vec!["feature-1".to_string(), "feature-2".to_string()]
        );
    }

    #[test]
    fn single_agent_override_preserves_branch_name() {
        let env = create_test_env();
        let specs = generate_worktree_specs(
            "feature",
            &[String::from("gemini")],
            None,
            None,
            &env,
            "{{ base_name }}{% if agent %}-{{ agent }}{% endif %}",
        )
        .expect("specs");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].branch_name, "feature");
        assert_eq!(specs[0].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_context_exposes_variables() {
        let env = create_test_env();
        let rows = parse_foreach_matrix("platform:ios,android;lang:swift,kotlin").expect("parse");
        let specs =
            generate_worktree_specs("feature", &[], None, Some(&rows), &env, "{{ base_name }}")
                .expect("specs");
        let rendered = env
            .render_str("{{ platform }}-{{ lang }}", &specs[0].template_context)
            .expect("prompt render");
        assert_eq!(rendered, "ios-swift");
    }

    #[test]
    fn render_prompt_template_inline_renders_variables() {
        let env = create_test_env();
        let mut context_map = JsonMap::new();
        context_map.insert(
            "branch".to_string(),
            JsonValue::String("feature-123".to_string()),
        );
        let context = JsonValue::Object(context_map);

        let prompt = Prompt::Inline("Working on {{ branch }}".to_string());
        let result = render_prompt_template(&prompt, &env, &context).expect("render success");

        match result {
            Prompt::Inline(text) => assert_eq!(text, "Working on feature-123"),
            _ => panic!("Expected Inline prompt"),
        }
    }

    #[test]
    fn render_prompt_template_from_file_reads_and_renders() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let env = create_test_env();
        let mut context_map = JsonMap::new();
        context_map.insert(
            "name".to_string(),
            JsonValue::String("test-branch".to_string()),
        );
        let context = JsonValue::Object(context_map);

        let mut temp_file = NamedTempFile::new().expect("create temp file");
        writeln!(temp_file, "Branch: {{{{ name }}}}").expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let result = render_prompt_template(&prompt, &env, &context).expect("render success");

        match result {
            Prompt::Inline(text) => assert_eq!(text, "Branch: test-branch\n"),
            _ => panic!("Expected Inline prompt"),
        }
    }

    #[test]
    fn render_prompt_template_from_nonexistent_file_fails() {
        let env = create_test_env();
        let context = JsonValue::Null;

        let prompt = Prompt::FromFile(PathBuf::from("/nonexistent/path/to/file.txt"));
        let result = render_prompt_template(&prompt, &env, &context);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read prompt file")
        );
    }

    #[test]
    fn branch_template_renders_with_foreach_vars() {
        let env = create_test_env();
        let mut foreach_vars = BTreeMap::new();
        foreach_vars.insert("platform".to_string(), "ios".to_string());
        foreach_vars.insert("lang".to_string(), "swift".to_string());

        let context = build_template_context("feature", &None, &None, &foreach_vars);
        // MiniJinja doesn't support unpacking in for loops, so we iterate over keys
        let template = "{{ base_name }}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}";
        let result = env.render_str(template, &context).expect("render");

        // The foreach_vars iteration should include both platform and lang values
        // BTreeMap is sorted, so lang comes before platform alphabetically
        assert_eq!(result, "feature-swift-ios");
    }

    #[test]
    fn foreach_with_agent_key_populates_spec_agent() {
        use crate::prompt::foreach_from_frontmatter;

        let env = create_test_env();
        let mut map = BTreeMap::new();
        map.insert(
            "agent".to_string(),
            vec!["claude".to_string(), "gemini".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");
        let specs = generate_worktree_specs(
            "feature",
            &[],
            None,
            Some(&rows),
            &env,
            "{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}",
        )
        .expect("specs");

        assert_eq!(specs.len(), 2);

        // First spec should have agent=claude and branch name should NOT include agent twice
        assert_eq!(specs[0].branch_name, "feature-claude");
        assert_eq!(specs[0].agent.as_deref(), Some("claude"));

        // Second spec should have agent=gemini
        assert_eq!(specs[1].branch_name, "feature-gemini");
        assert_eq!(specs[1].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_with_agent_and_other_vars_filters_agent_from_iteration() {
        use crate::prompt::foreach_from_frontmatter;

        let env = create_test_env();
        let mut map = BTreeMap::new();
        map.insert(
            "agent".to_string(),
            vec!["claude".to_string(), "gemini".to_string()],
        );
        map.insert(
            "platform".to_string(),
            vec!["ios".to_string(), "android".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");
        let specs = generate_worktree_specs(
            "feature",
            &[],
            None,
            Some(&rows),
            &env,
            "{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}",
        )
        .expect("specs");

        assert_eq!(specs.len(), 2);

        // Branch names should be: base-agent-platform (NOT base-agent-agent-platform or base-agent-platform-agent)
        // BTreeMap is sorted, so "agent" comes before "platform" alphabetically, but agent is filtered from foreach_vars
        assert_eq!(specs[0].branch_name, "feature-claude-ios");
        assert_eq!(specs[0].agent.as_deref(), Some("claude"));

        assert_eq!(specs[1].branch_name, "feature-gemini-android");
        assert_eq!(specs[1].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_filters_all_reserved_keys() {
        use crate::prompt::foreach_from_frontmatter;

        let env = create_test_env();
        let mut map = BTreeMap::new();
        // Try to use reserved keys in foreach
        map.insert(
            "base_name".to_string(),
            vec!["bad1".to_string(), "bad2".to_string()],
        );
        map.insert(
            "num".to_string(),
            vec!["bad3".to_string(), "bad4".to_string()],
        );
        map.insert(
            "foreach_vars".to_string(),
            vec!["bad5".to_string(), "bad6".to_string()],
        );
        map.insert(
            "agent".to_string(),
            vec!["bad7".to_string(), "bad8".to_string()],
        );
        map.insert(
            "platform".to_string(),
            vec!["ios".to_string(), "android".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");

        // Verify that reserved keys are NOT in the rows at the top level (only in the BTreeMap for lookup)
        // But the row itself should still contain them for extraction
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("platform").unwrap(), "ios");
        assert_eq!(rows[1].get("platform").unwrap(), "android");

        let specs = generate_worktree_specs(
            "base",
            &[],
            None,
            Some(&rows),
            &env,
            "{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}",
        )
        .expect("specs");

        // Branch name should only include platform, not the reserved keys
        // Reserved keys should be filtered from foreach_vars iteration
        assert_eq!(specs[0].branch_name, "base-bad7-ios");
        assert_eq!(specs[1].branch_name, "base-bad8-android");

        // base_name should be "base" (from function param), not "bad1" or "bad2"
        let context0 = &specs[0].template_context;
        assert_eq!(context0["base_name"].as_str().unwrap(), "base");

        // agent should be from foreach (bad7/bad8), not overwritten by reserved key collision
        assert_eq!(context0["agent"].as_str().unwrap(), "bad7");
    }

    // Helper function for tests
    fn render_prompt_template(
        prompt: &Prompt,
        env: &TemplateEnv,
        context: &JsonValue,
    ) -> Result<Prompt> {
        let template_str = match prompt {
            Prompt::Inline(text) => text.clone(),
            Prompt::FromFile(path) => fs::read_to_string(path)
                .with_context(|| format!("Failed to read prompt file '{}'", path.display()))?,
        };

        let rendered = env
            .render_str(&template_str, context)
            .context("Failed to render prompt template")?;
        Ok(Prompt::Inline(rendered))
    }
}
