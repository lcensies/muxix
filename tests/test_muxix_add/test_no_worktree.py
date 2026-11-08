"""`muxix add --no-worktree`: run the agent on the repository root checkout."""

from pathlib import Path

import pytest

from ..conftest import (
    MuxEnvironment,
    TmuxEnvironment,
    assert_window_exists,
    get_window_name,
    run_muxix_command,
    wait_for_window_ready,
    write_muxix_config,
)

# Reads the pane's cwd, which is a tmux-specific format string.
pytestmark = pytest.mark.tmux_only


def pane_cwd(env: MuxEnvironment, window_name: str) -> Path:
    assert isinstance(env, TmuxEnvironment)
    result = env.tmux(
        ["display-message", "-p", "-t", window_name, "#{pane_current_path}"]
    )
    return Path(result.stdout.strip())


def test_add_no_worktree_runs_at_the_repository_root(
    mux_server: MuxEnvironment, muxix_exe_path, mux_repo_path
):
    """No worktree and no branch are created; the pane opens at the repo root."""
    env = mux_server
    handle = "scratch-root"
    window_name = get_window_name(handle)

    write_muxix_config(mux_repo_path)

    worktrees_before = env.run_command(
        ["git", "worktree", "list"], cwd=mux_repo_path
    ).stdout.splitlines()

    run_muxix_command(env, muxix_exe_path, mux_repo_path, f"add {handle} --no-worktree")

    assert_window_exists(env, window_name)
    wait_for_window_ready(env, window_name)

    worktrees_after = env.run_command(
        ["git", "worktree", "list"], cwd=mux_repo_path
    ).stdout.splitlines()
    assert worktrees_after == worktrees_before, "no worktree may be added"

    branches = env.run_command(
        ["git", "branch", "--list", handle], cwd=mux_repo_path
    ).stdout
    assert branches.strip() == "", "no branch may be created"

    assert pane_cwd(env, window_name).resolve() == mux_repo_path.resolve()
