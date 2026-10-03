"""Tests for --max-concurrent worker pool functionality."""

from pathlib import Path


from ..conftest import (
    MuxEnvironment,
    run_muxix_command,
    write_muxix_config,
)


class TestMaxConcurrent:
    """Tests for --max-concurrent flag."""

    def test_max_concurrent_processes_sequentially(
        self,
        mux_server: MuxEnvironment,
        muxix_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies --max-concurrent limits parallel worktrees and processes queue."""
        env = mux_server

        # Configure pane to auto-close after a short delay (simulates agent completing)
        # Use 'exit' to close the pane/window in a backend-agnostic way
        write_muxix_config(mux_repo_path, panes=[{"command": "sleep 1 && exit"}])

        # 2 items with max-concurrent 1 = sequential processing
        # If worker pool works, this completes; if broken, it hangs forever
        run_muxix_command(
            env,
            muxix_exe_path,
            mux_repo_path,
            "add task --max-concurrent 1 --branch-template '{{ base_name }}-{{ index }}'",
            stdin_input="first\nsecond",
        )

        # Verify both worktrees were created (branches exist)
        for idx in [1, 2]:
            worktree_path = (
                mux_repo_path.parent
                / f"{mux_repo_path.name}__worktrees"
                / f"task-{idx}"
            )
            assert worktree_path.is_dir(), f"Expected worktree at {worktree_path}"
