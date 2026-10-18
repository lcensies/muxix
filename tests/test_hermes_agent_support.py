"""Black-box e2e tests for the hermes-agent-support change.

Each test corresponds to one `#### Scenario` heading in
openspec/changes/hermes-agent-support/specs/{agent-profiles,mcp-targets,agent-harness-coverage}/spec.md
"""

import json
from pathlib import Path

from .conftest import MuxEnvironment, run_muxix_command


def _hermes_home(mux_server: MuxEnvironment) -> Path:
    home = mux_server.home_path / ".hermes"
    home.mkdir(parents=True, exist_ok=True)
    return home


# ---------------------------------------------------------------------------
# agent-harness-coverage
# ---------------------------------------------------------------------------


def test_hermes_detection(mux_server: MuxEnvironment, muxix_exe_path, repo_path):
    """Scenario: Detection — HERMES_HOME naming an existing dir is detected."""
    hermes_home = _hermes_home(mux_server)
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup --check --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
        expect_fail=True,  # --check exits 2 when drift is reported
    )
    data = json.loads(result.stdout)
    assert any(i.get("agent") == "Hermes Agent" for i in data["items"])


def test_hermes_skills_installed(mux_server: MuxEnvironment, muxix_exe_path, repo_path):
    """Scenario: Skills — installed under HERMES_HOME/skills/<name>/."""
    hermes_home = _hermes_home(mux_server)
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only skills",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    skill_dirs = sorted(p.name for p in (hermes_home / "skills").iterdir())
    assert "muxix" in skill_dirs
    assert (hermes_home / "skills" / "muxix" / "SKILL.md").exists()


def test_hermes_settings_patch_is_yaml(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Settings patch is YAML — merged into config.yaml, preserving keys."""
    hermes_home = _hermes_home(mux_server)
    (hermes_home / "config.yaml").write_text("model: base-model\n")
    (repo_path / ".muxix.yaml").write_text(
        "bootstrap:\n  agents:\n    hermes:\n      settings:\n        temperature: 0.5\n"
    )
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only agent-settings",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    content = (hermes_home / "config.yaml").read_text()
    assert "temperature: 0.5" in content
    assert "model: base-model" in content
    # written as YAML, not JSON
    assert not content.strip().startswith("{")


def test_hermes_prompt_components_sentinel(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Prompt components — written into SOUL.md as a sentinel block."""
    hermes_home = _hermes_home(mux_server)
    (hermes_home / "SOUL.md").write_text("# My personal soul file\nI like cats.\n")
    pc_dir = repo_path / ".muxix" / "prompt-components"
    pc_dir.mkdir(parents=True)
    (pc_dir / "house-style.md").write_text("Use tabs not spaces.")
    (repo_path / ".muxix.yaml").write_text(
        "bootstrap:\n  prompt_components:\n    - house-style\n"
    )
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only prompts",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    soul = (hermes_home / "SOUL.md").read_text()
    assert "I like cats." in soul
    assert "Use tabs not spaces." in soul
    assert "muxix-bootstrap-begin" in soul


def test_hermes_no_subagent_definitions(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: No subagent definitions — hermes is skipped with a reason."""
    hermes_home = _hermes_home(mux_server)
    (repo_path / ".muxix.yaml").write_text(
        "bootstrap:\n"
        "  subagents:\n"
        "    - name: planner\n"
        "      description: Plans work\n"
        "      prompt: You plan things.\n"
    )
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup --check --json --only subagents",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    data = json.loads(result.stdout)
    hermes_items = [i for i in data["items"] if i.get("agent") == "Hermes Agent"]
    assert len(hermes_items) == 1
    assert hermes_items[0]["outcome"] == "skipped"
    assert "runtime-only" in hermes_items[0]["detail"]


def test_hermes_hook_install(mux_server: MuxEnvironment, muxix_exe_path, repo_path):
    """Scenario: Hook install — hooks: block merged mapping events to muxix set-window-status."""
    hermes_home = _hermes_home(mux_server)
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    content = (hermes_home / "config.yaml").read_text()
    assert "pre_llm_call" in content
    assert "post_tool_call" in content
    assert "pre_approval_request" in content
    assert "on_session_end" in content
    assert "muxix set-window-status working" in content
    assert "muxix set-window-status waiting" in content
    assert "muxix set-window-status done" in content


def test_hermes_only_valid_shell_hook_events(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Only valid shell-hook events are written — never gateway event names."""
    hermes_home = _hermes_home(mux_server)
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    content = (hermes_home / "config.yaml").read_text()
    for gateway_event in ("agent:start", "agent:end", "agent:step"):
        assert gateway_event not in content


def test_hermes_consent_granted_and_reported(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Consent is granted and reported — hooks_auto_accept set + reported."""
    hermes_home = _hermes_home(mux_server)
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    assert "hooks_auto_accept: true" in (hermes_home / "config.yaml").read_text()
    data = json.loads(result.stdout)
    hermes_item = next(i for i in data["items"] if i.get("agent") == "Hermes Agent")
    assert "hooks_auto_accept" in hermes_item["detail"]
    assert "auto-approves" in hermes_item["detail"]


def test_hermes_already_configured(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Already configured — check reports installed, writes nothing."""
    hermes_home = _hermes_home(mux_server)
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    before = (hermes_home / "config.yaml").read_text()
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup --check --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    after = (hermes_home / "config.yaml").read_text()
    assert before == after
    data = json.loads(result.stdout)
    hermes_item = next(i for i in data["items"] if i.get("agent") == "Hermes Agent")
    assert hermes_item["outcome"] == "up-to-date"
    assert data["exitCode"] == 0


def test_hermes_stale_or_partial(mux_server: MuxEnvironment, muxix_exe_path, repo_path):
    """Scenario: Stale or partial — missing hooks_auto_accept reports needing install."""
    hermes_home = _hermes_home(mux_server)
    (hermes_home / "config.yaml").write_text(
        "hooks:\n"
        "  on_session_end:\n"
        "  - command: muxix set-window-status done\n"
        "  post_tool_call:\n"
        "  - command: muxix set-window-status working\n"
        "  pre_approval_request:\n"
        "  - command: muxix set-window-status waiting\n"
        "  pre_llm_call:\n"
        "  - command: muxix set-window-status working\n"
        "hooks_auto_accept: false\n"
    )
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup --check --json --only hooks",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
        expect_fail=True,
    )
    data = json.loads(result.stdout)
    hermes_item = next(i for i in data["items"] if i.get("agent") == "Hermes Agent")
    assert hermes_item["outcome"] != "up-to-date"
    assert data["exitCode"] == 2


def test_hermes_nix_managed_config_readonly(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Nix-managed config — read-only config.yaml fails with an actionable message."""
    hermes_home = _hermes_home(mux_server)
    cfg = hermes_home / "config.yaml"
    cfg.write_text("hooks_auto_accept: false\n")
    cfg.chmod(0o444)
    try:
        result = run_muxix_command(
            mux_server,
            muxix_exe_path,
            repo_path,
            "setup -y --json --only hooks",
            pre_run_env={"HERMES_HOME": str(hermes_home)},
            expect_fail=True,
        )
    finally:
        cfg.chmod(0o644)
    data = json.loads(result.stdout)
    hermes_item = next(i for i in data["items"] if i.get("agent") == "Hermes Agent")
    assert hermes_item["outcome"] == "failed"
    assert str(cfg) in hermes_item["detail"]
    assert "read-only" in hermes_item["detail"]
    assert "EACCES" not in hermes_item["detail"]


# ---------------------------------------------------------------------------
# mcp-targets
# ---------------------------------------------------------------------------


def test_hermes_mcp_servers_declared_detected(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Servers declared with hermes detected — no file written for hermes,
    report lists hermes as unsupported with a reason.
    """
    hermes_home = _hermes_home(mux_server)
    (repo_path / ".muxix.yaml").write_text(
        "mcp:\n  context7:\n    command: context7-mcp\n"
    )
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only mcp",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    data = json.loads(result.stdout)
    hermes_items = [i for i in data["items"] if i.get("agent") == "Hermes Agent"]
    assert hermes_items, (
        "expected an item reporting Hermes Agent as an unsupported mcp target, "
        f"got: {data['items']}"
    )
    assert hermes_items[0]["outcome"] == "skipped"
    assert not (repo_path / ".mcp.json").exists()


def test_hermes_mcp_another_agent_shares_worktree(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Another agent shares the worktree — Claude's .mcp.json still
    converges, and hermes' unsupported status does not fail the sync.
    """
    hermes_home = _hermes_home(mux_server)
    (mux_server.home_path / ".claude").mkdir(parents=True, exist_ok=True)
    (repo_path / ".muxix.yaml").write_text(
        "mcp:\n  context7:\n    command: context7-mcp\n"
    )
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --only mcp",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    assert result.exit_code == 0
    assert (repo_path / ".mcp.json").exists()
    mcp_json = json.loads((repo_path / ".mcp.json").read_text())
    assert "context7" in mcp_json["mcpServers"]


# ---------------------------------------------------------------------------
# agent-profiles
# ---------------------------------------------------------------------------


def test_hermes_profiled_launch(mux_server: MuxEnvironment, muxix_exe_path, repo_path):
    """Scenario: Profiled launch — setup builds an overlay, exec points HERMES_HOME at it."""
    hermes_home = _hermes_home(mux_server)
    (hermes_home / "config.yaml").write_text("model: base-model\n")
    (repo_path / ".muxix.yaml").write_text(
        "profiles:\n"
        "  work: {}\n"
        "agent_profiles:\n"
        "  work:\n"
        "    agents:\n"
        "      hermes:\n"
        "        settings:\n"
        "          foo: bar\n"
    )
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --profile work --only agent-profiles",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    overlay = (
        mux_server.home_path
        / ".local"
        / "state"
        / "muxix"
        / "agent-profiles"
        / "work"
        / "hermes"
        / "config.yaml"
    )
    assert overlay.exists()

    fake_hermes = mux_server.fake_bin_dir / "hermes"
    fake_hermes.write_text('#!/bin/sh\necho "HERMES_HOME=$HERMES_HOME"\n')
    fake_hermes.chmod(0o755)
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "exec --profile work hermes",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    assert f"HERMES_HOME={overlay.parent}" in result.stdout


def test_hermes_settings_deltas_written_as_yaml(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: Settings deltas are written as YAML — overlay config.yaml is YAML."""
    hermes_home = _hermes_home(mux_server)
    (hermes_home / "config.yaml").write_text("model: base-model\n")
    (repo_path / ".muxix.yaml").write_text(
        "profiles:\n"
        "  work: {}\n"
        "agent_profiles:\n"
        "  work:\n"
        "    agents:\n"
        "      hermes:\n"
        "        settings:\n"
        "          foo: bar\n"
    )
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --profile work --only agent-profiles",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    overlay = (
        mux_server.home_path
        / ".local"
        / "state"
        / "muxix"
        / "agent-profiles"
        / "work"
        / "hermes"
        / "config.yaml"
    )
    content = overlay.read_text()
    assert "foo: bar" in content
    assert "model: base-model" in content
    assert not content.strip().startswith("{")


def test_hermes_no_session_dir_redirect(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: No session-dir redirect — only HERMES_HOME is set, no separate
    session-dir env var.
    """
    hermes_home = _hermes_home(mux_server)
    (hermes_home / "config.yaml").write_text("model: base-model\n")
    (repo_path / ".muxix.yaml").write_text(
        "profiles:\n"
        "  work: {}\n"
        "agent_profiles:\n"
        "  work:\n"
        "    agents:\n"
        "      hermes:\n"
        "        settings:\n"
        "          foo: bar\n"
    )
    run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "setup -y --json --profile work --only agent-profiles",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    fake_hermes = mux_server.fake_bin_dir / "hermes"
    fake_hermes.write_text("#!/bin/sh\nenv | grep -i SESSION_DIR || true\n")
    fake_hermes.chmod(0o755)
    result = run_muxix_command(
        mux_server,
        muxix_exe_path,
        repo_path,
        "exec --profile work hermes",
        pre_run_env={"HERMES_HOME": str(hermes_home)},
    )
    assert "SESSION_DIR" not in result.stdout


def test_hermes_sticky_profile_documented(
    mux_server: MuxEnvironment, muxix_exe_path, repo_path
):
    """Scenario: A sticky hermes profile takes precedence — hermes' own
    active_profile overrides HERMES_HOME; the docs state this so the overlay
    is not reported as broken. The re-homing itself is hermes' own behavior
    (not muxix's), so this test only verifies muxix documents it.
    """
    docs_text = (
        Path(__file__).parent.parent / "docs" / "guide" / "profiles.md"
    ).read_text()
    assert "active_profile" in docs_text
    assert "re-homes" in docs_text
