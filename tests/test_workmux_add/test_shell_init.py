"""Tests for shell initialization and login shell behavior."""

import pytest

from ..conftest import (
    MuxEnvironment,
    poll_until,
    wait_for_file,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


# WezTerm: CLI spawn doesn't support passing environment variables to spawned
# panes, so we can't set a test HOME to verify .bash_profile is sourced.
@pytest.mark.tmux_only
class TestLoginShell:
    """Tests that workmux starts shells as login shells."""

    def test_panes_start_as_login_shells(
        self, mux_server: MuxEnvironment, workmux_exe_path, repo_path
    ):
        """
        Verifies that panes are started as login shells by checking if
        .bash_profile is sourced.
        """
        env = mux_server
        branch_name = "feature-login-shell"
        marker_file = env.home_path / "profile_loaded"

        # 1. Configure bash as the shell
        # We use bash because its login shell behavior (.bash_profile) is standard
        bash_path = "/bin/bash"
        env.configure_default_shell(bash_path)

        # 2. Create .bash_profile that creates a marker file
        # This file is only sourced if bash is started as a login shell (e.g. bash -l)
        bash_profile = env.home_path / ".bash_profile"
        bash_profile.write_text(f"touch {marker_file}\n")

        # 3. Create workmux config with a command
        # A command is required to trigger the wrapper logic in setup_panes
        write_workmux_config(repo_path, panes=[{"command": "echo 'starting pane'"}])

        # 4. Run workmux add
        add_branch_and_get_worktree(env, workmux_exe_path, repo_path, branch_name)

        # 5. Wait for marker file
        # This confirms that the shell executed the profile
        wait_for_file(env, marker_file, timeout=5.0)

    def test_split_panes_start_as_login_shells(
        self, mux_server: MuxEnvironment, workmux_exe_path, repo_path
    ):
        """
        Verifies that split panes are also started as login shells.
        """
        env = mux_server
        branch_name = "feature-split-login"
        log_file = env.home_path / "profile_log"

        # 1. Configure bash
        bash_path = "/bin/bash"
        env.configure_default_shell(bash_path)

        # 2. Create .bash_profile that appends to a log
        bash_profile = env.home_path / ".bash_profile"
        bash_profile.write_text(f"echo 'loaded' >> {log_file}\n")

        # 3. Create workmux config with two panes (one initial, one split)
        write_workmux_config(
            repo_path,
            panes=[
                {"command": "echo pane1"},
                {"split": "horizontal", "command": "echo pane2"},
            ],
        )

        # 4. Run workmux add
        add_branch_and_get_worktree(env, workmux_exe_path, repo_path, branch_name)

        # 5. Wait for log file to have 2 lines (one for each pane)
        def check_log_lines():
            if not log_file.exists():
                return False
            content = log_file.read_text()
            return content.strip().count("loaded") >= 2

        assert poll_until(check_log_lines, timeout=5.0), (
            f"Expected 2 login shells, log content:\n"
            f"{log_file.read_text() if log_file.exists() else 'File not found'}"
        )
