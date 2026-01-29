"""Tests for shell initialization and login shell behavior."""

import pytest

from ..conftest import (
    MuxEnvironment,
    ShellCommands,
    poll_until,
    wait_for_file,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


# WezTerm: CLI spawn doesn't support passing environment variables to spawned
# panes, so we can't set a test HOME to verify RC files are sourced.
@pytest.mark.tmux_only
class TestLoginShell:
    """Tests that workmux starts shells as login shells."""

    def test_panes_start_as_login_shells(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path,
        repo_path,
        shell_cmd: ShellCommands,
    ):
        """
        Verifies that panes are started as login shells by checking if
        the shell's RC file is sourced.
        """
        env = mux_server
        branch_name = "feature-login-shell"
        marker_file = env.home_path / "profile_loaded"

        # 1. Configure the shell
        env.configure_default_shell(shell_cmd.path)

        # 2. Create RC file that creates a marker file
        # This file is only sourced if the shell starts properly
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_path.write_text(f"touch {marker_file}\n")

        # 3. Create workmux config with a command
        # A command is required to trigger the wrapper logic in setup_panes
        write_workmux_config(repo_path, panes=[{"command": "echo 'starting pane'"}])

        # 4. Run workmux add
        add_branch_and_get_worktree(env, workmux_exe_path, repo_path, branch_name)

        # 5. Wait for marker file
        # This confirms that the shell executed the RC file
        wait_for_file(env, marker_file, timeout=5.0)

    def test_split_panes_start_as_login_shells(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path,
        repo_path,
        shell_cmd: ShellCommands,
    ):
        """
        Verifies that split panes are also started as login shells.
        """
        env = mux_server
        branch_name = "feature-split-login"
        log_file = env.home_path / "profile_log"

        # 1. Configure the shell
        env.configure_default_shell(shell_cmd.path)

        # 2. Create RC file that appends to a log
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_path.write_text(shell_cmd.append_to_file("loaded", str(log_file)) + "\n")

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
