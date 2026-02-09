"""
Tests for `workmux wait` command.

Tests error handling, timeout behavior, and waiting for real agent state.
"""

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


def test_wait_error_worktree_not_found(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Wait fails when worktree doesn't exist."""
    result = run_workmux_command(
        mux_server,
        workmux_exe_path,
        mux_repo_path,
        "wait nonexistent --timeout 1",
        expect_fail=True,
    )
    assert result.exit_code != 0


def test_wait_error_invalid_status(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Wait fails with invalid status value."""
    env = mux_server
    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, "feature-wait-inv")

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "wait feature-wait-inv --status invalid --timeout 1",
        expect_fail=True,
    )
    assert "Invalid status" in result.stderr


def test_wait_timeout_exits_with_code_1(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Wait with --timeout exits with code 1 when timeout is reached."""
    env = mux_server
    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, "feature-wait-to")
    time.sleep(1.5)

    # Create agent state with "working" status
    window_name = get_window_name("feature-wait-to")
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Wait for "done" but agent is "working", so it should timeout
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "wait feature-wait-to --status done --timeout 2",
        expect_fail=True,
    )
    assert result.exit_code == 1
    assert "Timeout" in result.stderr


def test_wait_succeeds_when_already_in_target_status(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Wait returns immediately when agent is already in the target status."""
    env = mux_server
    branch_name = "feature-wait-ok"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create agent state with "done" status
    status_cmd = build_status_cmd(env, workmux_exe_path, "done")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Wait for "done" - should succeed immediately
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "wait feature-wait-ok --status done --timeout 5",
    )
    assert result.exit_code == 0
