"""
Tests for `workmux capture` command.

Tests error paths and happy-path capture from a live agent pane.
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


def test_capture_error_worktree_not_found(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Capture fails when worktree doesn't exist."""
    result = run_workmux_command(
        mux_server,
        workmux_exe_path,
        mux_repo_path,
        "capture nonexistent",
        expect_fail=True,
    )
    assert result.exit_code != 0


def test_capture_error_no_agent_in_worktree(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Capture fails when worktree exists but no agent is running."""
    env = mux_server
    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, "feature-no-agent")

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "capture feature-no-agent",
        expect_fail=True,
    )
    assert "No agent running" in result.stderr


def test_capture_output_from_agent(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Capture returns pane content from a running agent."""
    env = mux_server
    branch_name = "feature-capture"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Put some recognizable text in the pane
    env.send_keys(window_name, "echo CAPTURE_MARKER_12345")
    time.sleep(0.5)

    # Create real agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Capture the pane
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "capture feature-capture",
    )
    assert result.exit_code == 0
    assert "CAPTURE_MARKER_12345" in result.stdout


def test_capture_strips_ansi(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Capture output has ANSI escape codes stripped."""
    env = mux_server
    branch_name = "feature-capture-ansi"
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

    # Capture the pane - output should not contain ANSI escape sequences
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "capture feature-capture-ansi",
    )
    assert result.exit_code == 0
    # The output should be clean text without escape codes
    assert "\x1b[" not in result.stdout


def test_capture_custom_line_count(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Capture respects the --lines flag."""
    env = mux_server
    branch_name = "feature-capture-lines"
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

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "capture feature-capture-lines -n 10",
    )
    assert result.exit_code == 0
