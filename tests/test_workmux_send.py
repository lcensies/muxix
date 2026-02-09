"""
Tests for `workmux send` command.

Tests error paths and argument validation. Happy-path tests (sending text to
a live agent pane) require a reconciled agent with matching backend/instance,
which is set up via set-window-status.
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


def test_send_error_worktree_not_found(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Send fails with helpful error when worktree doesn't exist."""
    result = run_workmux_command(
        mux_server,
        workmux_exe_path,
        mux_repo_path,
        "send nonexistent hello",
        expect_fail=True,
    )
    assert result.exit_code != 0


def test_send_error_no_agent_in_worktree(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Send fails when worktree exists but no agent is running."""
    env = mux_server
    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, "feature-no-agent")

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "send feature-no-agent hello",
        expect_fail=True,
    )
    assert "No agent running" in result.stderr


def test_send_error_text_and_file_conflict(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Send fails when both text and --file are provided (clap conflict)."""
    result = run_workmux_command(
        mux_server,
        workmux_exe_path,
        mux_repo_path,
        "send some-wt hello --file /tmp/foo.txt",
        expect_fail=True,
    )
    assert result.exit_code != 0


def test_send_inline_text_to_agent(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Send delivers inline text to a running agent's pane."""
    env = mux_server
    branch_name = "feature-send-text"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create real agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "waiting")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Send a simple single-word text (no shell quoting issues)
    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "send feature-send-text hello-from-send",
    )
    assert result.exit_code == 0


def test_send_from_file_to_agent(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Send delivers file content to a running agent's pane."""
    env = mux_server
    branch_name = "feature-send-file"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create real agent state
    status_cmd = build_status_cmd(env, workmux_exe_path, "waiting")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Create a file with content to send in /tmp (short path to avoid
    # tmux line-length truncation with long PATH env var in test harness)
    prompt_file = Path("/tmp/wm_prompt.txt")
    prompt_file.write_text("hello-from-file\n")

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        f"send feature-send-file --file {prompt_file}",
    )
    assert result.exit_code == 0
