"""Tests for recovery from stale registered worktree metadata."""

from pathlib import Path

from ..conftest import (
    MuxEnvironment,
    get_window_name,
    get_worktree_path,
    run_muxix_command,
    write_muxix_config,
)


class TestStaleWorktreeRecovery:
    """Tests for recreating a worktree whose directory is missing but is still
    registered in git's worktree metadata (e.g. after cloning a repo from a
    machine that had the linked worktrees)."""

    def test_add_recovers_missing_registered_worktree(
        self,
        mux_server: MuxEnvironment,
        muxix_exe_path: Path,
        repo_path: Path,
    ):
        """`muxix add` should prune stale metadata and recreate a missing worktree."""
        env = mux_server
        branch_name = "feature-stale-worktree"
        worktree_path = get_worktree_path(repo_path, branch_name)

        write_muxix_config(repo_path)

        # Create the worktree and window, then close the window (keeping the worktree).
        run_muxix_command(
            env,
            muxix_exe_path,
            repo_path,
            f"add {branch_name} --background",
        )
        assert worktree_path.is_dir()
        assert (worktree_path / ".git").exists()

        run_muxix_command(
            env,
            muxix_exe_path,
            repo_path,
            f"close {branch_name}",
        )

        # Simulate fetching the repo on a machine without the linked worktree directories.
        env.run_command(["rm", "-rf", str(worktree_path)], cwd=repo_path)
        assert not worktree_path.exists()

        # The branch should still be registered to the missing path.
        list_before = env.run_command(
            ["git", "worktree", "list"],
            cwd=repo_path,
        )
        assert branch_name in list_before.stdout

        # Recreating the worktree should succeed by pruning stale metadata.
        run_muxix_command(
            env,
            muxix_exe_path,
            repo_path,
            f"add {branch_name} --background",
        )

        assert worktree_path.is_dir()
        assert (worktree_path / ".git").exists()

        list_after = env.run_command(
            ["git", "worktree", "list"],
            cwd=repo_path,
        )
        assert branch_name in list_after.stdout

        # The window should also have been recreated.
        window_name = get_window_name(branch_name)
        assert window_name in env.list_windows()
