"""Tests for post_create hooks and pane commands in `workmux add`."""

from pathlib import Path


from ..conftest import (
    MuxEnvironment,
    ShellCommands,
    get_window_name,
    wait_for_pane_output,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestPostCreateHooks:
    """Tests for post_create hook execution."""

    def test_add_executes_post_create_hooks(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies that `workmux add` executes post_create hooks in the worktree directory."""
        env = mux_server
        branch_name = "feature-hooks"
        hook_file = "hook_was_executed.txt"

        write_workmux_config(mux_repo_path, post_create=[f"touch {hook_file}"])

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )

        # Verify hook file was created in the worktree directory
        assert (worktree_path / hook_file).exists()

    def test_add_can_skip_post_create_hooks(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """`workmux add --no-hooks` should not run configured post_create hooks."""
        env = mux_server
        branch_name = "feature-skip-hooks"
        hook_file = "hook_should_not_exist.txt"

        write_workmux_config(mux_repo_path, post_create=[f"touch {hook_file}"])

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args="--no-hooks",
        )

        assert not (worktree_path / hook_file).exists()


class TestPaneCommands:
    """Tests for pane command execution."""

    def test_add_executes_pane_commands(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies that `workmux add` executes commands in configured panes."""
        env = mux_server
        branch_name = "feature-panes"
        window_name = get_window_name(branch_name)
        expected_output = "test pane command output"

        write_workmux_config(
            mux_repo_path, panes=[{"command": f"echo '{expected_output}'; sleep 0.5"}]
        )

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        wait_for_pane_output(env, window_name, expected_output)

    def test_add_can_skip_pane_commands(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """`workmux add --no-pane-cmds` should create panes without running commands."""
        env = mux_server
        branch_name = "feature-skip-pane-cmds"
        marker_file = "pane_command_output.txt"

        write_workmux_config(mux_repo_path, panes=[{"command": f"touch {marker_file}"}])

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args="--no-pane-cmds",
        )

        assert not (worktree_path / marker_file).exists()


class TestShellRcFiles:
    """Tests for shell rc file sourcing."""

    def test_add_sources_shell_rc_files(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        shell_cmd: ShellCommands,
    ):
        """Verifies that pane commands run in a shell that has sourced its rc file."""
        env = mux_server
        branch_name = "feature-aliases"
        window_name = get_window_name(branch_name)
        alias_output = "custom_alias_worked_correctly"

        # Configure the default shell
        env.configure_default_shell(shell_cmd.path)

        # Write the appropriate RC file for this shell
        rc_path = env.home_path / shell_cmd.rc_filename
        rc_path.parent.mkdir(parents=True, exist_ok=True)
        rc_content = f"""
# Test alias
{shell_cmd.alias("testcmd", f'echo "{alias_output}"')}
"""
        rc_path.write_text(rc_content)

        write_workmux_config(mux_repo_path, panes=[{"command": "testcmd"}])

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        wait_for_pane_output(
            env,
            window_name,
            alias_output,
            timeout=5.0,  # Increased for slower shells like nushell
        )
