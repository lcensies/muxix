use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

/// Show the current policy status (offline — reads cached policy.yaml).
pub fn run_status() -> Result<()> {
    use crate::provision::cache::{CacheStatus, load_policy};

    let grace = 72 * 3600;
    match load_policy(grace)? {
        CacheStatus::Fresh(policy) => {
            println!("org policy: fresh");
            println!("  version: {}", policy.policy_version);
            println!("  issued:  {}", policy.issued_at);
            println!("  expires: {}", policy.expires_at);
            if !policy.forbidden_mcp_commands.is_empty() {
                println!(
                    "  forbidden mcp patterns: {}",
                    policy.forbidden_mcp_commands.join(", ")
                );
            }
            if !policy.allowed_agent_kinds.is_empty() {
                println!(
                    "  allowed agents: {}",
                    policy.allowed_agent_kinds.join(", ")
                );
            }
        }
        CacheStatus::Stale(policy) => {
            eprintln!(
                "warning: org policy is stale (version {}). Run 'muxix provision sync' to refresh.",
                policy.policy_version
            );
        }
        CacheStatus::Expired => {
            eprintln!("error: org policy has expired. Run 'muxix provision sync' to re-sync.");
        }
        CacheStatus::Missing => {
            println!("org policy: not configured");
            println!("  Run 'muxix provision sync' after setting the MUXIX_PROVISION_URL env var");
            println!("  (or provision.server_url in your global config).");
        }
    }
    Ok(())
}

/// Full sync: push profile snapshot → fetch policy → cache → audit.
pub fn run_sync(dry_run: bool, strict: bool) -> Result<()> {
    use crate::config::Config;
    use crate::provision::types::AuditEvent;
    use crate::provision::{agents, audit, cache, merge, profile, source};

    let config = Config::load(None).unwrap_or_default();
    let provision_cfg = config.provision.clone().unwrap_or_default();

    // The backend decides where the policy comes from: an HTTPS server, a file
    // dropped on disk, or a helper the org already trusts. Everything below is
    // written against the trait, not against HTTP.
    let backend = source::Backend::from_config(&provision_cfg)?;
    let policy_source = source::source_for(&provision_cfg)?;
    let origin = policy_source.describe();

    println!(
        "provision sync: backend = {}, source = {}",
        backend.as_str(),
        origin
    );

    let snapshot = profile::generate_snapshot(&config);
    println!(
        "provision sync: profile muxix/{} on {}",
        snapshot.muxix_version, snapshot.platform
    );

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let mut audit_result = "ok".to_string();
    let mut lock_overrides = 0usize;

    if !dry_run {
        match policy_source.push_profile(&snapshot) {
            // A backend with nowhere to send a profile (file, exec, or an HTTP
            // server with `endpoints.profile: null`) is not an error.
            Ok(None) => {}
            Ok(Some(resp)) => {
                if resp.accepted {
                    println!(
                        "provision sync: profile accepted (id={:?})",
                        resp.profile_id
                    );
                    if !resp.policy_needs_update {
                        // Don't short-circuit: we still fetch the policy below and
                        // re-assert agent config from it every sync (a cheap GET),
                        // so a freshly-provisioned machine gets its agents wired
                        // even when the profile push was a no-op.
                        println!("provision sync: policy is up to date");
                    }
                } else {
                    println!(
                        "provision sync: profile skipped ({})",
                        resp.reason.as_deref().unwrap_or("identical_checksum")
                    );
                }
            }
            Err(e) => {
                audit_result = format!("push error: {}", e);
                eprintln!("warning: provision push failed: {}", e);
                // Non-fatal — still try to fetch policy
            }
        }
    }

    // Fetch policy
    let policy_resp = match policy_source.fetch_policy().and_then(|r| {
        // Reject a policy this build cannot interpret before anything is cached
        // or applied from it.
        source::check_schema_version(&r)?;
        Ok(r)
    }) {
        Ok(r) => {
            println!("provision sync: fetched policy version {}", r.version);
            r
        }
        Err(e) => {
            audit_result = format!("fetch error: {}", e);
            let _ = audit::append_audit_event(&AuditEvent {
                timestamp: now_secs.to_string(),
                event_type: "sync".into(),
                server_url: Some(origin.clone()),
                policy_version: None,
                policy_checksum: None,
                violations: 0,
                backend: Some(backend.as_str().to_string()),
                lock_overrides,
                result: audit_result,
            });
            if strict {
                return Err(e);
            }
            eprintln!("warning: {}", e);
            return Ok(());
        }
    };

    // Stamp TTL timestamps
    let mut policy = policy_resp.policy;
    policy.issued_at = now_secs;
    policy.expires_at = now_secs + policy_resp.ttl_seconds;
    policy.checksum = policy_resp.checksum.clone();

    // Name every value the policy's locks will override. This is the one place
    // a user is actively syncing policy, so it is where the report belongs --
    // doing it on every config load would cost a second resolve on the hot path.
    if let Some(locks) = crate::provision::layers::locks_layer(&policy) {
        let current = serde_yaml::to_value(&config).unwrap_or_default();
        let overridden = crate::provision::layers::overrides(&current, &locks);
        lock_overrides = overridden.len();
        for path in &overridden {
            println!(
                "provision sync: {} is locked by org policy and overrides your setting",
                path
            );
        }
    }

    // Validate against current config
    let violations = merge::validate_policy(&config, &policy);
    let violations_count = violations.len();
    if !violations.is_empty() {
        merge::report_violations(&violations);
        if strict {
            anyhow::bail!(
                "{} policy violation(s) — failing due to --strict",
                violations_count
            );
        }
    } else {
        println!("provision sync: config is compliant");
    }

    if dry_run {
        println!("provision sync: dry-run — no changes written");
        return Ok(());
    }

    cache::save_policy(&policy)?;
    println!(
        "provision sync: policy cached ({})",
        cache::policy_path()?.display()
    );

    // Point local coding agents (opencode, Claude Code) at the governed gateway,
    // so their traffic routes through the org's gateway with no manual env wiring. The
    // gateway is a sibling of the policy in the fetch response (policy_resp),
    // not a field of the cached policy.
    if let Some(gw) = &policy_resp.gateway {
        // `origin` is the fallback base URL when the gateway names none; for a
        // file or exec backend there is no URL to fall back to, and
        // apply_gateway then relies on the gateway's own base_url.
        match agents::apply_gateway(gw, &origin) {
            Ok(written) => {
                for w in written {
                    println!("provision sync: configured {}", w);
                }
            }
            Err(e) => eprintln!("warning: agent config not written: {}", e),
        }
    }

    let _ = audit::append_audit_event(&AuditEvent {
        timestamp: now_secs.to_string(),
        event_type: "sync".into(),
        server_url: Some(origin.clone()),
        policy_version: Some(policy.policy_version.clone()),
        policy_checksum: Some(policy_resp.checksum),
        violations: violations_count,
        backend: Some(backend.as_str().to_string()),
        lock_overrides,
        result: audit_result,
    });

    Ok(())
}

/// Dry-run: show what policy would be applied without modifying anything.
#[allow(dead_code)]
pub fn run_dry_run() -> Result<()> {
    use crate::provision::cache::{CacheStatus, load_policy};

    let grace = 72 * 3600;
    match load_policy(grace)? {
        CacheStatus::Fresh(policy) | CacheStatus::Stale(policy) => {
            println!(
                "dry-run: org policy v{} would be applied",
                policy.policy_version
            );
            if policy.locked.proxy_chain.is_some() {
                println!("  locked: proxy_chain");
            }
            if policy.locked.sandbox_network.is_some() {
                println!("  locked: sandbox.network");
            }
            if !policy.forbidden_mcp_commands.is_empty() {
                println!(
                    "  forbidden mcp patterns: {}",
                    policy.forbidden_mcp_commands.join(", ")
                );
            }
        }
        CacheStatus::Expired => {
            eprintln!("org policy has expired — run 'muxix provision sync'");
        }
        CacheStatus::Missing => {
            println!("no cached policy found — nothing to apply");
        }
    }
    Ok(())
}
