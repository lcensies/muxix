"""
Multi-shell compatibility tests.

These tests verify that workmux works correctly with different shells.
Run with TEST_ALL_SHELLS=1 to test all available shells.
"""

from pathlib import Path

from ..conftest import (
    MuxEnvironment,
    ShellCommands,
    get_window_name,
    wait_for_pane_output,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestShellCompatibility:
    """Tests for shell-specific compatibility."""

    def test_pane_command_executes_in_configured_shell(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        shell_cmd: ShellCommands,
    ):
        """Verifies pane commands execute in the configured shell."""
        env = mux_server
        branch_name = "test-shell-exec"
        window_name = get_window_name(branch_name)

        env.configure_default_shell(shell_cmd.path)

        # Use a command that works in all shells
        marker = f"shell_is_{shell_cmd.name}"
        write_workmux_config(
            mux_repo_path,
            panes=[{"command": f"echo {marker}"}],
        )

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        wait_for_pane_output(env, window_name, marker, timeout=5.0)

    def test_env_var_from_rc_file(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        shell_cmd: ShellCommands,
    ):
        """Verifies environment variables from RC files work in pane commands."""
        env = mux_server
        branch_name = "test-env-var"
        window_name = get_window_name(branch_name)

        env.configure_default_shell(shell_cmd.path)

        # Write RC file that sets an env var
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_path.write_text(shell_cmd.set_env("TEST_MARKER", "env_var_works") + "\n")

        # Use shell-specific env var reference syntax
        write_workmux_config(
            mux_repo_path,
            panes=[{"command": f"echo {shell_cmd.env_ref('TEST_MARKER')}"}],
        )

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        wait_for_pane_output(env, window_name, "env_var_works", timeout=5.0)
