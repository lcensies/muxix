"""Tests for pre_merge and pre_remove hooks in `workmux merge`."""

from pathlib import Path

from .conftest import (
    MuxEnvironment,
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
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that pre_merge hooks run when merging a worktree."""
        env = mux_server
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
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies all environment variables are set correctly during merge."""
        env = mux_server
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
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that a failing pre_merge hook aborts the merge."""
        env = mux_server
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

    def test_pre_merge_hook_skipped_with_no_verify(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that pre_merge hooks are skipped when --no-verify is passed."""
        env = mux_server
        branch_name = "feature-no-verify"
        marker_file = env.tmp_path / "should_not_exist.txt"

        # Configure a hook that creates a file
        write_workmux_config(
            repo_path,
            pre_merge=[f"touch {marker_file}"],
            env=env,
        )

        run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
        worktree_path = get_worktree_path(repo_path, branch_name)
        create_commit(env, worktree_path, "feat: test commit")

        # Run merge with --no-verify
        run_workmux_merge(env, workmux_exe_path, repo_path, branch_name, no_verify=True)

        assert not marker_file.exists(), "Hook should NOT have run with --no-verify"
        assert not worktree_path.exists(), "Merge should still complete successfully"

    def test_no_verify_bypasses_failing_hook(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that --no-verify allows merge to succeed even with a failing hook configured."""
        env = mux_server
        branch_name = "feature-bypass-fail"

        # Configure a hook that would fail
        write_workmux_config(
            repo_path,
            pre_merge=["exit 1"],
            env=env,
        )

        run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
        worktree_path = get_worktree_path(repo_path, branch_name)
        create_commit(env, worktree_path, "feat: test commit")

        # Merge should succeed with --no-verify despite the failing hook
        run_workmux_merge(env, workmux_exe_path, repo_path, branch_name, no_verify=True)

        assert not worktree_path.exists(), (
            "Merge should complete successfully with --no-verify"
        )

    def test_no_hooks_skips_pre_merge_and_pre_remove(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that --no-hooks skips both pre_merge and pre_remove hooks."""
        env = mux_server
        branch_name = "feature-no-hooks"
        pre_merge_marker = env.tmp_path / "pre_merge_ran.txt"
        pre_remove_marker = env.tmp_path / "pre_remove_ran.txt"

        write_workmux_config(
            repo_path,
            pre_merge=[f"touch {pre_merge_marker}"],
            pre_remove=[f"touch {pre_remove_marker}"],
            env=env,
        )

        run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
        worktree_path = get_worktree_path(repo_path, branch_name)
        create_commit(env, worktree_path, "feat: test commit")

        run_workmux_merge(env, workmux_exe_path, repo_path, branch_name, no_hooks=True)

        assert not pre_merge_marker.exists(), (
            "pre_merge hook should NOT have run with --no-hooks"
        )
        assert not pre_remove_marker.exists(), (
            "pre_remove hook should NOT have run with --no-hooks"
        )
        assert not worktree_path.exists(), (
            "Merge should still complete successfully with --no-hooks"
        )
