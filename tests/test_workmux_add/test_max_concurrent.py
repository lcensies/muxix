"""Tests for --max-concurrent worker pool functionality."""

from pathlib import Path

from ..conftest import (
    TmuxEnvironment,
    run_workmux_command,
    write_workmux_config,
)


class TestMaxConcurrent:
    """Tests for --max-concurrent flag."""

    def test_max_concurrent_processes_sequentially(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies --max-concurrent limits parallel worktrees and processes queue."""
        env = isolated_tmux_server

        # Configure pane to auto-close after a short delay (simulates agent completing)
        write_workmux_config(
            repo_path, panes=[{"command": "sleep 1 && tmux kill-window"}]
        )

        # 2 items with max-concurrent 1 = sequential processing
        # If worker pool works, this completes; if broken, it hangs forever
        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add task --max-concurrent 1 --branch-template '{{ base_name }}-{{ index }}'",
            stdin_input="first\nsecond",
        )

        # Verify both worktrees were created (branches exist)
        for idx in [1, 2]:
            worktree_path = (
                repo_path.parent / f"{repo_path.name}__worktrees" / f"task-{idx}"
            )
            assert worktree_path.is_dir(), f"Expected worktree at {worktree_path}"
