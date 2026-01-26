"""
Basic workmux add tests that work with both tmux and WezTerm backends.

Run with:
    WORKMUX_TEST_BACKEND=wezterm pytest tests/test_workmux_add/test_basic_multibackend.py -v
    WORKMUX_TEST_BACKEND=tmux pytest tests/test_workmux_add/test_basic_multibackend.py -v
    pytest --backend=wezterm tests/test_workmux_add/test_basic_multibackend.py -v
"""

from pathlib import Path

import pytest

from ..conftest import (
    MuxEnvironment,
    assert_window_exists,
    get_window_name,
    get_worktree_path,
    run_workmux_command,
    setup_git_repo,
    write_workmux_config,
)


@pytest.fixture
def repo_path(mux_server: MuxEnvironment) -> Path:
    """Initialize a git repo in the test env and return its path."""
    path = mux_server.tmp_path
    setup_git_repo(path, mux_server.env)
    return path


class TestWorktreeCreation:
    """Tests for basic worktree creation with workmux add."""

    def test_add_creates_worktree(
        self, mux_server: MuxEnvironment, workmux_exe_path: Path, repo_path: Path
    ):
        """workmux add should create a new git worktree."""
        branch_name = "test-feature"

        write_workmux_config(repo_path)
        run_workmux_command(
            mux_server, workmux_exe_path, repo_path, f"add {branch_name}"
        )

        expected_path = get_worktree_path(repo_path, branch_name)
        assert expected_path.exists(), f"Worktree not created at {expected_path}"
        assert (expected_path / ".git").exists(), "Worktree missing .git"

    def test_add_creates_window(
        self, mux_server: MuxEnvironment, workmux_exe_path: Path, repo_path: Path
    ):
        """workmux add should create a new multiplexer window/tab."""
        branch_name = "test-feature"

        write_workmux_config(repo_path)
        run_workmux_command(
            mux_server, workmux_exe_path, repo_path, f"add {branch_name}"
        )

        expected_window = get_window_name(branch_name)
        assert_window_exists(mux_server, expected_window)
