"""Tests for orphan directory cleanup during `workmux add`."""

from ..conftest import (
    get_worktree_path,
    run_workmux_command,
    run_workmux_remove,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestOrphanDirectoryCleanup:
    """Tests for auto-removal of orphan directories when creating worktrees."""

    def test_add_removes_empty_orphan_directory(
        self, mux_server, workmux_exe_path, repo_path
    ):
        """
        Verifies that `workmux add` removes an empty orphan directory
        (exists on disk but not registered with git).
        """
        env = mux_server
        branch_name = "feature-orphan-empty"

        write_workmux_config(repo_path)

        # Create the orphan directory structure (simulating what a background
        # process might create after cleanup renames the worktree)
        worktree_path = get_worktree_path(repo_path, branch_name)
        worktree_path.mkdir(parents=True)

        # Verify it's not registered with git
        result = env.run_command(
            ["git", "worktree", "list", "--porcelain"], cwd=repo_path
        )
        assert str(worktree_path) not in result.stdout

        # workmux add should succeed by removing the orphan directory
        actual_path = add_branch_and_get_worktree(
            env, workmux_exe_path, repo_path, branch_name
        )

        # Verify worktree was created successfully
        assert actual_path.is_dir()
        assert (actual_path / ".git").exists()

        # Verify it's now registered with git
        result = env.run_command(["git", "worktree", "list"], cwd=repo_path)
        assert branch_name in result.stdout

    def test_add_removes_orphan_directory_with_nested_empty_dirs(
        self, mux_server, workmux_exe_path, repo_path
    ):
        """
        Verifies that `workmux add` removes an orphan directory containing
        only nested empty subdirectories (common when build tools recreate paths).
        """
        env = mux_server
        branch_name = "feature-orphan-nested"

        write_workmux_config(repo_path)

        # Create orphan directory with nested empty structure
        # (simulating mkdir -p from a build tool with stale $PWD)
        worktree_path = get_worktree_path(repo_path, branch_name)
        (worktree_path / "workspaces" / "web" / ".next" / "types").mkdir(parents=True)

        # workmux add should succeed
        actual_path = add_branch_and_get_worktree(
            env, workmux_exe_path, repo_path, branch_name
        )

        assert actual_path.is_dir()
        assert (actual_path / ".git").exists()

    def test_add_removes_orphan_directory_with_build_artifacts(
        self, mux_server, workmux_exe_path, repo_path
    ):
        """
        Verifies that `workmux add` removes an orphan directory containing
        build artifacts (files created by background processes after cleanup).
        """
        env = mux_server
        branch_name = "feature-orphan-artifacts"

        write_workmux_config(repo_path)

        # Create orphan directory with build artifacts
        worktree_path = get_worktree_path(repo_path, branch_name)
        next_types_dir = worktree_path / "workspaces" / "web" / ".next" / "types"
        next_types_dir.mkdir(parents=True)

        # Create some build artifact files
        (next_types_dir / "routes.d.ts").write_text("// generated routes")
        (next_types_dir / "validator.ts").write_text("// generated validator")

        # workmux add should succeed by removing the orphan directory
        actual_path = add_branch_and_get_worktree(
            env, workmux_exe_path, repo_path, branch_name
        )

        assert actual_path.is_dir()
        assert (actual_path / ".git").exists()

    def test_add_fails_for_orphan_directory_with_git_resource(
        self, mux_server, workmux_exe_path, repo_path
    ):
        """
        Verifies that `workmux add` refuses to remove an orphan directory
        that contains a .git file/folder (could be corrupted worktree or manual clone).
        """
        env = mux_server
        branch_name = "feature-orphan-git"

        write_workmux_config(repo_path)

        # Create orphan directory with a .git file (simulating corrupted worktree)
        worktree_path = get_worktree_path(repo_path, branch_name)
        worktree_path.mkdir(parents=True)
        (worktree_path / ".git").write_text("gitdir: /some/path/.git/worktrees/foo")

        # workmux add should fail with a clear error
        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            f"add {branch_name}",
            expect_fail=True,
        )

        assert "contains a .git resource" in result.stderr
        assert "not registered" in result.stderr

    def test_add_fails_for_registered_worktree(
        self, mux_server, workmux_exe_path, repo_path
    ):
        """
        Verifies that `workmux add` still fails properly when the directory
        is a registered git worktree (not an orphan).
        """
        env = mux_server
        branch_name = "feature-registered"

        write_workmux_config(repo_path)

        # Create worktree the normal way first
        add_branch_and_get_worktree(env, workmux_exe_path, repo_path, branch_name)

        # Try to create another worktree with the same branch
        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            f"add {branch_name}",
            expect_fail=True,
        )

        assert "already exists" in result.stderr

    def test_add_after_remove_with_orphan_recreation(
        self, mux_server, workmux_exe_path, repo_path
    ):
        """
        End-to-end test: create worktree, remove it, simulate orphan recreation,
        then verify add succeeds again.
        """
        env = mux_server
        branch_name = "feature-full-cycle"

        write_workmux_config(repo_path)

        # 1. Create worktree
        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, repo_path, branch_name
        )
        assert worktree_path.is_dir()

        # 2. Remove the worktree
        run_workmux_remove(env, workmux_exe_path, repo_path, branch_name, force=True)

        # Verify it's gone
        assert not worktree_path.exists()

        # 3. Simulate orphan recreation (background process recreates directory)
        orphan_dir = worktree_path / "node_modules" / ".cache"
        orphan_dir.mkdir(parents=True)
        (orphan_dir / "cache.json").write_text("{}")

        # Verify orphan exists but is not registered
        assert worktree_path.exists()
        result = env.run_command(["git", "worktree", "list"], cwd=repo_path)
        assert str(worktree_path) not in result.stdout

        # 4. workmux add should succeed by cleaning up the orphan
        new_worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, repo_path, branch_name
        )

        assert new_worktree_path.is_dir()
        assert (new_worktree_path / ".git").exists()

        # Verify the orphan artifacts are gone
        assert not (new_worktree_path / "node_modules").exists()
