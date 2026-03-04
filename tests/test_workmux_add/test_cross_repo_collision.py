"""Tests for cross-repo window name collision handling.

When the same branch name is used across different repos in the same tmux
session, workmux auto-suffixes the handle with the project directory name
to avoid window name collisions.
"""

from pathlib import Path

from ..conftest import (
    DEFAULT_WINDOW_PREFIX,
    MuxEnvironment,
    assert_window_exists,
    get_window_name,
    run_workmux_command,
    run_workmux_remove,
    setup_git_repo,
    slugify,
    write_workmux_config,
)


def _create_second_repo(env: MuxEnvironment, name: str = "second-repo") -> Path:
    """Create a second git repo as a sibling to the main repo."""
    second_repo = env.tmp_path.parent / name
    second_repo.mkdir(parents=True, exist_ok=True)
    setup_git_repo(second_repo, env.env)
    return second_repo


class TestCrossRepoCollision:
    """Tests for cross-repo window name collision auto-suffix."""

    def test_same_branch_in_two_repos_succeeds(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Second `wm add` with same branch name in different repo succeeds with project-name suffix."""
        env = mux_server
        branch = "feature-auth"

        # Setup first repo and create worktree
        write_workmux_config(mux_repo_path)
        run_workmux_command(env, workmux_exe_path, mux_repo_path, f"add {branch}")

        # Verify first window exists
        first_window = get_window_name(branch)
        assert_window_exists(env, first_window)

        # Setup second repo and create worktree with same branch
        second_repo = _create_second_repo(env)
        write_workmux_config(second_repo)
        result = run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch}",
            working_dir=second_repo,
        )

        # Should succeed (no error)
        assert result.exit_code == 0

    def test_suffixed_window_name_contains_project(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """The auto-suffixed window name includes the project directory name."""
        env = mux_server
        branch = "feature-login"
        second_repo_name = "second-repo"

        # First repo
        write_workmux_config(mux_repo_path)
        run_workmux_command(env, workmux_exe_path, mux_repo_path, f"add {branch}")

        # Second repo with known name
        second_repo = _create_second_repo(env, name=second_repo_name)
        write_workmux_config(second_repo)
        run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch}",
            working_dir=second_repo,
        )

        # The suffixed window should exist
        handle = slugify(branch)
        suffixed_window = f"{DEFAULT_WINDOW_PREFIX}{handle}-{slugify(second_repo_name)}"
        assert_window_exists(env, suffixed_window)

        # Original window should still exist too
        assert_window_exists(env, get_window_name(branch))

    def test_remove_one_repo_does_not_kill_other(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Removing worktree in one repo does NOT kill the other repo's window."""
        env = mux_server
        branch = "feature-shared"

        # First repo
        write_workmux_config(mux_repo_path)
        run_workmux_command(env, workmux_exe_path, mux_repo_path, f"add {branch}")
        first_window = get_window_name(branch)

        # Second repo
        second_repo = _create_second_repo(env)
        write_workmux_config(second_repo)
        run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch}",
            working_dir=second_repo,
        )

        # Remove from second repo
        run_workmux_remove(env, workmux_exe_path, second_repo, branch, force=True)

        # First repo's window should still exist
        assert_window_exists(env, first_window)

    def test_explicit_name_bypasses_auto_suffix(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """--name flag overrides auto-suffix behavior."""
        env = mux_server
        branch = "feature-override"
        custom_name = "my-custom-name"

        # First repo
        write_workmux_config(mux_repo_path)
        run_workmux_command(env, workmux_exe_path, mux_repo_path, f"add {branch}")

        # Second repo with explicit --name
        second_repo = _create_second_repo(env)
        write_workmux_config(second_repo)
        run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch} --name {custom_name}",
            working_dir=second_repo,
        )

        # Custom-named window should exist
        custom_window = f"{DEFAULT_WINDOW_PREFIX}{custom_name}"
        assert_window_exists(env, custom_window)

    def test_stderr_shows_info_message(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Auto-suffix prints an informational message to stderr."""
        env = mux_server
        branch = "feature-info"

        # First repo
        write_workmux_config(mux_repo_path)
        run_workmux_command(env, workmux_exe_path, mux_repo_path, f"add {branch}")

        # Second repo
        second_repo = _create_second_repo(env)
        write_workmux_config(second_repo)
        result = run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch}",
            working_dir=second_repo,
        )

        assert "exists in another repository" in result.stderr

    def test_numeric_repo_name_uses_safe_suffix(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Numeric repo names get a 'repo-' prefix to avoid cleanup regex collision."""
        env = mux_server
        branch = "feature-numeric"

        # First repo
        write_workmux_config(mux_repo_path)
        run_workmux_command(env, workmux_exe_path, mux_repo_path, f"add {branch}")
        first_window = get_window_name(branch)

        # Second repo with purely numeric name
        second_repo = _create_second_repo(env, name="123")
        write_workmux_config(second_repo)
        run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch}",
            working_dir=second_repo,
        )

        # Suffix should be "repo-123" not just "123" (which would match cleanup regex)
        handle = slugify(branch)
        suffixed_window = f"{DEFAULT_WINDOW_PREFIX}{handle}-repo-123"
        assert_window_exists(env, suffixed_window)

        # Remove from second repo should NOT kill first repo's window
        run_workmux_remove(env, workmux_exe_path, second_repo, branch, force=True)
        assert_window_exists(env, first_window)

    def test_explicit_name_collision_fails(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """When --name collides cross-repo, it fails instead of auto-suffixing."""
        env = mux_server
        branch_a = "feature-a"
        branch_b = "feature-b"
        shared_name = "shared-handle"

        # First repo with explicit name
        write_workmux_config(mux_repo_path)
        run_workmux_command(
            env, workmux_exe_path, mux_repo_path, f"add {branch_a} --name {shared_name}"
        )

        # Second repo with same explicit name should fail
        second_repo = _create_second_repo(env)
        write_workmux_config(second_repo)
        result = run_workmux_command(
            env,
            workmux_exe_path,
            second_repo,
            f"add {branch_b} --name {shared_name}",
            working_dir=second_repo,
            expect_fail=True,
        )

        assert "already exists" in result.stderr
