"""
Tests for `workmux status` command.

Tests output format, JSON mode, filtering, and behavior with real agent state.
"""

import json
import time
from pathlib import Path

from .conftest import (
    MuxEnvironment,
    get_window_name,
    poll_until,
    run_workmux_add,
    run_workmux_command,
    write_workmux_config,
)

from .test_agent_state import build_status_cmd, list_agent_state_files


def test_status_no_agents(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Status shows 'No active agents' when no agents are running."""
    result = run_workmux_command(mux_server, workmux_exe_path, mux_repo_path, "status")
    assert "No active agents" in result.stdout


def test_status_json_no_agents(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Status --json returns empty array when no agents are running."""
    result = run_workmux_command(
        mux_server, workmux_exe_path, mux_repo_path, "status --json"
    )
    parsed = json.loads(result.stdout)
    assert parsed == []


def test_status_with_active_agent(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Status shows agent info when an agent is active."""
    env = mux_server
    branch_name = "feature-status-active"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create real agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    result = run_workmux_command(env, workmux_exe_path, mux_repo_path, "status")
    # Should show the worktree and its status
    assert "working" in result.stdout
    assert "WORKTREE" in result.stdout
    assert "STATUS" in result.stdout


def test_status_json_with_active_agent(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Status --json returns agent data when an agent is active."""
    env = mux_server
    branch_name = "feature-status-json"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create real agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "done")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    result = run_workmux_command(env, workmux_exe_path, mux_repo_path, "status --json")
    parsed = json.loads(result.stdout)
    assert isinstance(parsed, list)
    assert len(parsed) >= 1

    entry = parsed[0]
    assert "worktree" in entry
    assert "branch" in entry
    assert "status" in entry
    assert "pane_id" in entry
    assert entry["status"] == "done"


def test_status_filter_by_worktree(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Status filters to show only the specified worktree."""
    env = mux_server
    branch_name = "feature-status-filt"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Filter by this worktree's branch name - should return it
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        f"status --json {branch_name}",
    )
    parsed = json.loads(result.stdout)
    assert len(parsed) >= 1
    # All results should be for the filtered worktree
    for entry in parsed:
        assert entry["branch"] == branch_name


def test_status_filter_no_match(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Status with a filter that matches no agents shows 'No active agents'."""
    env = mux_server
    branch_name = "feature-status-exists"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Filter by a name that doesn't match any agent
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "status nonexistent-worktree",
    )
    assert "No active agents" in result.stdout
