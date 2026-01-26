"""Basic tests for `workmux add` command - worktree creation and flags."""

import pytest

from ..conftest import (
    MuxEnvironment,
    assert_window_exists,
    create_commit,
    file_for_commit,
    get_window_name,
    get_worktree_path,
    run_workmux_add,
    run_workmux_command,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestWorktreeCreation:
    """Tests for basic worktree creation functionality."""

    def test_add_creates_worktree(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Verifies that `workmux add` creates a git worktree."""
        env = mux_server
        branch_name = "feature-worktree"

        write_workmux_config(mux_repo_path)

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )

        # Verify worktree in git's state
        worktree_list_result = env.run_command(["git", "worktree", "list"])
        assert branch_name in worktree_list_result.stdout

        # Verify worktree directory exists
        assert worktree_path.is_dir()

    def test_add_creates_mux_window(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Verifies that `workmux add` creates a multiplexer window with the correct name."""
        env = mux_server
        branch_name = "feature-window"
        window_name = get_window_name(branch_name)

        write_workmux_config(mux_repo_path)

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        assert_window_exists(env, window_name)

    def test_add_from_inside_worktree_creates_sibling_worktree(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """
        Verifies that running `workmux add` from inside an existing worktree
        creates the new worktree as a sibling to the repo, not nested inside the current worktree.
        """
        env = mux_server
        first_branch = "feature-first"
        second_branch = "feature-second"

        write_workmux_config(mux_repo_path)

        # 1. Create the first worktree normally
        first_worktree = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, first_branch
        )

        # Create a unique commit in the first worktree to verify ancestry
        commit_msg = "Commit in first worktree"
        create_commit(env, first_worktree, commit_msg)

        # 2. Run `workmux add` for the second branch FROM INSIDE the first worktree
        add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            second_branch,
            working_dir=first_worktree,
        )

        # 3. Verify the second worktree exists at the correct sibling location
        expected_path = get_worktree_path(mux_repo_path, second_branch)
        assert expected_path.exists(), f"Worktree should be created at {expected_path}"
        assert expected_path.is_dir()

        # 4. Verify it is NOT nested inside the first worktree
        nested_path = (
            first_worktree.parent / f"{first_worktree.name}__worktrees" / second_branch
        )
        assert not nested_path.exists(), (
            f"Worktree should not be nested at {nested_path}"
        )

        # 5. Verify ancestry: The second branch should be based on the first branch
        expected_file = file_for_commit(expected_path, commit_msg)
        assert expected_file.exists(), (
            "New branch should inherit commit from the current worktree context"
        )

        # 6. Verify git internal state is correct
        worktree_list = env.run_command(
            ["git", "worktree", "list"], cwd=mux_repo_path
        ).stdout
        assert str(expected_path) in worktree_list


class TestCountFlag:
    """Tests for the -n/--count flag."""

    def test_add_with_count_creates_numbered_worktrees(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Verifies `-n` spawns multiple numbered worktrees."""
        env = mux_server
        base_name = "feature-counted"

        write_workmux_config(mux_repo_path)
        run_workmux_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            f"add {base_name} -n 2",
        )

        for idx in (1, 2):
            branch = f"{base_name}-{idx}"
            worktree = get_worktree_path(mux_repo_path, branch)
            assert worktree.is_dir()
            assert_window_exists(env, get_window_name(branch))


class TestBaseFlag:
    """Tests for the --base flag."""

    def test_add_from_specific_branch(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Verifies that `workmux add --base` creates a worktree from a specific branch."""
        env = mux_server
        new_branch = "feature-from-base"

        write_workmux_config(mux_repo_path)

        # Create a commit on the current branch
        create_commit(env, mux_repo_path, "Add base file")

        # Get current branch name
        result = env.run_command(["git", "branch", "--show-current"], cwd=mux_repo_path)
        base_branch = result.stdout.strip()

        # Run workmux add with --base flag
        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            new_branch,
            extra_args=f"--base {base_branch}",
        )

        # Verify the new branch contains the file from base branch
        expected_file = file_for_commit(worktree_path, "Add base file")
        assert expected_file.exists()

        # Verify multiplexer window was created
        window_name = get_window_name(new_branch)
        assert_window_exists(env, window_name)

    def test_add_defaults_to_current_branch(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """`workmux add` without --base should inherit from the current branch."""
        env = mux_server
        base_branch = "feature-default-base"
        stacked_branch = "feature-default-child"
        commit_message = "Stack default change"

        write_workmux_config(mux_repo_path)

        env.run_command(["git", "checkout", "-b", base_branch], cwd=mux_repo_path)
        create_commit(env, mux_repo_path, commit_message)

        stacked_worktree = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, stacked_branch
        )
        expected_file = file_for_commit(stacked_worktree, commit_message)
        assert expected_file.exists()

        window_name = get_window_name(stacked_branch)
        assert_window_exists(env, window_name)

    def test_add_with_base_does_not_set_upstream(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path,
        mux_repo_path,
        mux_remote_repo_path,
    ):
        """Verifies that `--base origin/main` does not set origin/main as upstream."""
        env = mux_server
        new_branch = "feature-no-upstream"

        write_workmux_config(mux_repo_path)

        # Set up remote and push main
        env.run_command(
            ["git", "remote", "add", "origin", str(mux_remote_repo_path)],
            cwd=mux_repo_path,
        )
        env.run_command(["git", "push", "-u", "origin", "main"], cwd=mux_repo_path)

        # Create a new branch based on origin/main
        add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            new_branch,
            extra_args="--base origin/main",
        )

        # Verify NO upstream config remains (neither merge nor remote)
        for key in ["merge", "remote"]:
            result = env.run_command(
                ["git", "config", "--get", f"branch.{new_branch}.{key}"],
                cwd=mux_repo_path,
                check=False,
            )
            assert result.returncode != 0, (
                f"Branch '{new_branch}' should not have 'branch.{new_branch}.{key}' set, "
                f"but found: {result.stdout.strip()}"
            )


class TestDetachedHead:
    """Tests for behavior with detached HEAD states."""

    def test_add_errors_when_detached_head_without_base(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Detached HEAD states should require --base."""
        env = mux_server
        branch_name = "feature-detached-head"

        write_workmux_config(mux_repo_path)

        head_sha = env.run_command(
            ["git", "rev-parse", "HEAD"], cwd=mux_repo_path
        ).stdout.strip()
        env.run_command(["git", "checkout", head_sha], cwd=mux_repo_path)

        result = run_workmux_command(
            env, workmux_exe_path, mux_repo_path, f"add {branch_name}", expect_fail=True
        )

        assert "detached HEAD" in result.stderr

    def test_add_allows_detached_head_with_explicit_base(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Detached HEAD states can still create worktrees when --base is provided."""
        env = mux_server
        branch_name = "feature-detached-head-base"
        commit_message = "Detached baseline"

        write_workmux_config(mux_repo_path)
        create_commit(env, mux_repo_path, commit_message)

        head_sha = env.run_command(
            ["git", "rev-parse", "HEAD"], cwd=mux_repo_path
        ).stdout.strip()
        env.run_command(["git", "checkout", head_sha], cwd=mux_repo_path)

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args="--base main",
        )

        expected_file = file_for_commit(worktree_path, commit_message)
        assert expected_file.exists()


class TestExistingBranch:
    """Tests for behavior with existing branches."""

    def test_add_reuses_existing_branch(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Verifies that `workmux add` reuses an existing branch instead of creating a new one."""
        env = mux_server
        branch_name = "feature-existing-branch"
        commit_message = "Existing branch changes"

        write_workmux_config(mux_repo_path)

        # Remember the default branch so we can switch back after preparing the feature branch
        current_branch_result = env.run_command(
            ["git", "branch", "--show-current"], cwd=mux_repo_path
        )
        default_branch = current_branch_result.stdout.strip()

        # Create and populate an existing branch
        env.run_command(["git", "checkout", "-b", branch_name], cwd=mux_repo_path)
        create_commit(env, mux_repo_path, commit_message)
        branch_head = env.run_command(
            ["git", "rev-parse", "HEAD"], cwd=mux_repo_path
        ).stdout.strip()

        # Switch back to the default branch so workmux add runs from a typical state
        env.run_command(["git", "checkout", default_branch], cwd=mux_repo_path)

        worktree_path = add_branch_and_get_worktree(
            env, workmux_exe_path, mux_repo_path, branch_name
        )
        expected_file = file_for_commit(worktree_path, commit_message)
        assert expected_file.exists()
        assert expected_file.read_text() == f"content for {commit_message}"

        # The branch should still point to the commit we created earlier
        branch_tip = env.run_command(
            ["git", "rev-parse", branch_name], cwd=mux_repo_path
        ).stdout.strip()
        assert branch_tip == branch_head

    def test_add_fails_when_worktree_exists(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path
    ):
        """Verifies that `workmux add` fails with a clear message if the worktree already exists."""
        env = mux_server
        branch_name = "feature-existing-worktree"
        existing_worktree_path = mux_repo_path.parent / "existing_worktree_dir"

        write_workmux_config(mux_repo_path)

        # Create the branch and then return to the default branch
        env.run_command(["git", "checkout", "-b", branch_name], cwd=mux_repo_path)
        env.run_command(["git", "checkout", "main"], cwd=mux_repo_path)

        # Manually create a git worktree for the branch to simulate the pre-existing state
        env.run_command(
            ["git", "worktree", "add", str(existing_worktree_path), branch_name],
            cwd=mux_repo_path,
        )

        with pytest.raises(AssertionError) as excinfo:
            run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

        stderr = str(excinfo.value)
        assert f"A worktree for branch '{branch_name}' already exists." in stderr
        assert "Use 'workmux open" in stderr


class TestRemoteBranch:
    """Tests for behavior with remote branches."""

    def test_add_from_remote_branch(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path,
        mux_repo_path,
        mux_remote_repo_path,
    ):
        """When the branch exists only on the remote, workmux add should fetch and track it."""
        env = mux_server
        remote_branch_path = "feature/remote-pr"
        remote_ref = f"origin/{remote_branch_path}"
        commit_message = "Remote PR work"

        write_workmux_config(mux_repo_path)

        # Wire up the repo to a bare remote and push the default branch.
        env.run_command(
            ["git", "remote", "add", "origin", str(mux_remote_repo_path)],
            cwd=mux_repo_path,
        )
        env.run_command(["git", "push", "-u", "origin", "main"], cwd=mux_repo_path)

        # Create a branch with commits and push it to the remote.
        env.run_command(
            ["git", "checkout", "-b", remote_branch_path], cwd=mux_repo_path
        )
        create_commit(env, mux_repo_path, commit_message)
        remote_tip = env.run_command(
            ["git", "rev-parse", remote_branch_path], cwd=mux_repo_path
        ).stdout.strip()
        env.run_command(
            ["git", "push", "-u", "origin", remote_branch_path], cwd=mux_repo_path
        )

        # Remove the local branch and remote-tracking ref so the branch only exists on the remote.
        env.run_command(["git", "checkout", "main"], cwd=mux_repo_path)
        env.run_command(["git", "branch", "-D", remote_branch_path], cwd=mux_repo_path)
        env.run_command(
            ["git", "update-ref", "-d", f"refs/remotes/{remote_ref}"],
            cwd=mux_repo_path,
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            remote_branch_path,
            command_target=remote_ref,
        )
        expected_file = file_for_commit(worktree_path, commit_message)
        assert expected_file.exists()
        assert expected_file.read_text() == f"content for {commit_message}"

        # Local branch should point to the remote commit and track origin/<branch_name>.
        branch_tip = env.run_command(
            ["git", "rev-parse", remote_branch_path], cwd=mux_repo_path
        ).stdout.strip()
        assert branch_tip == remote_tip

        upstream_tip = env.run_command(
            ["git", "rev-parse", f"{remote_branch_path}@{{upstream}}"],
            cwd=mux_repo_path,
        ).stdout.strip()
        assert upstream_tip == remote_tip

        origin_tip = env.run_command(
            ["git", "rev-parse", remote_ref], cwd=mux_repo_path
        ).stdout.strip()
        assert origin_tip == remote_tip


# WezTerm: get_current_window() returns None because WezTerm doesn't expose
# session-level window focus via CLI - GUI focus is controlled by window manager.
@pytest.mark.tmux_only
class TestBackgroundFlag:
    """Tests for the --background flag."""

    def test_add_background_creates_window_without_switching(
        self, mux_server: MuxEnvironment, workmux_exe_path, repo_path
    ):
        """Verifies that `workmux add --background` creates window without switching to it."""
        env = mux_server
        branch_name = "feature-background"
        initial_window = "initial"

        write_workmux_config(repo_path)

        # Create an initial window and remember it
        env.new_window(initial_window)
        env.select_window(initial_window)

        # Get current window before running add
        current_before = env.get_current_window()
        assert current_before == initial_window

        # Run workmux add with --background flag
        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            repo_path,
            branch_name,
            extra_args="--background",
        )

        # Verify worktree was created
        assert worktree_path.is_dir()

        # Verify the new window exists
        window_name = get_window_name(branch_name)
        assert_window_exists(env, window_name)

        # Verify we're still on the initial window (didn't switch)
        current_after = env.get_current_window()
        assert current_after == initial_window
