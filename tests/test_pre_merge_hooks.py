"""Tests for pre_merge hooks in `workmux merge`."""

from pathlib import Path

from .conftest import (
    TmuxEnvironment,
    get_worktree_path,
    run_workmux_add,
    run_workmux_merge,
    write_workmux_config,
    create_commit,
)


class TestPreMergeHooks:
    """Tests for pre_merge hook execution during `workmux merge`."""

    def test_pre_merge_hook_runs_on_merge(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that pre_merge hooks run when merging a worktree."""
        env = isolated_tmux_server
        branch_name = "feature-pre-merge"
        marker_file = env.tmp_path / "pre_merge_ran.txt"

        write_workmux_config(
            repo_path,
            pre_merge=[f"touch {marker_file}"],
            env=env,
        )

        run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
        worktree_path = get_worktree_path(repo_path, branch_name)
        create_commit(env, worktree_path, "feat: test commit")

        run_workmux_merge(env, workmux_exe_path, repo_path, branch_name)

        assert marker_file.exists(), "pre_merge hook should have created marker file"
        assert not worktree_path.exists(), "Worktree should be removed after merge"

    def test_pre_merge_hook_receives_all_env_vars(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies all environment variables are set correctly during merge."""
        env = isolated_tmux_server
        branch_name = "feature-all-env"
        env_file = env.tmp_path / "merge_hook_env.txt"

        write_workmux_config(
            repo_path,
            pre_merge=[
                f'echo "BRANCH=$WM_BRANCH_NAME" >> {env_file}',
                f'echo "TARGET=$WM_TARGET_BRANCH" >> {env_file}',
                f'echo "PATH=$WM_WORKTREE_PATH" >> {env_file}',
                f'echo "ROOT=$WM_PROJECT_ROOT" >> {env_file}',
                f'echo "HANDLE=$WM_HANDLE" >> {env_file}',
            ],
            env=env,
        )

        run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
        expected_worktree = get_worktree_path(repo_path, branch_name)
        create_commit(env, expected_worktree, "feat: test commit")

        run_workmux_merge(env, workmux_exe_path, repo_path, branch_name)

        assert env_file.exists(), "Hook should have written environment variables"
        content = env_file.read_text()
        assert f"BRANCH={branch_name}" in content
        assert "TARGET=main" in content
        assert f"PATH={expected_worktree}" in content
        assert f"ROOT={repo_path}" in content
        assert f"HANDLE={branch_name}" in content

    def test_pre_merge_hook_failure_aborts_merge(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that a failing pre_merge hook aborts the merge."""
        env = isolated_tmux_server
        branch_name = "feature-fail-hook"

        write_workmux_config(
            repo_path,
            pre_merge=["exit 1"],
            env=env,
        )

        run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
        worktree_path = get_worktree_path(repo_path, branch_name)
        create_commit(env, worktree_path, "feat: test commit")

        run_workmux_merge(
            env, workmux_exe_path, repo_path, branch_name, expect_fail=True
        )

        assert worktree_path.exists(), "Worktree should NOT be removed when hook fails"
