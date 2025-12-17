import uuid
from pathlib import Path


from .conftest import (
    DEFAULT_WINDOW_PREFIX,
    TmuxEnvironment,
    create_commit,
    create_dirty_file,
    get_window_name,
    get_worktree_path,
    run_workmux_add,
    run_workmux_remove,
    write_workmux_config,
)


def test_remove_clean_branch_succeeds_without_prompt(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove` on a branch with no unmerged commits succeeds without a prompt."""
    env = isolated_tmux_server
    branch_name = "clean-branch"
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path)

    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
    worktree_path = get_worktree_path(repo_path, branch_name)
    assert worktree_path.is_dir()

    # This should succeed without any user input because the branch has no new commits
    run_workmux_remove(env, workmux_exe_path, repo_path, branch_name, force=False)

    assert not worktree_path.exists()
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_name not in list_windows_result.stdout
    branch_list_result = env.run_command(["git", "branch", "--list", branch_name])
    assert branch_name not in branch_list_result.stdout


def test_remove_unmerged_branch_with_confirmation(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove` on an unmerged branch succeeds after user confirmation."""
    env = isolated_tmux_server
    branch_name = "unmerged-branch"
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Create a new commit to make the branch "unmerged"
    worktree_path = get_worktree_path(repo_path, branch_name)
    create_commit(env, worktree_path, "feat: new feature")

    # Run remove, piping 'y' to the confirmation prompt
    run_workmux_remove(
        env, workmux_exe_path, repo_path, branch_name, force=False, user_input="y"
    )

    assert not worktree_path.exists(), "Worktree should be removed after confirmation"
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_name not in list_windows_result.stdout
    branch_list_result = env.run_command(["git", "branch", "--list", branch_name])
    assert branch_name not in branch_list_result.stdout


def test_remove_unmerged_branch_aborted(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove` on an unmerged branch is aborted if not confirmed."""
    env = isolated_tmux_server
    branch_name = "unmerged-aborted"
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_commit(env, worktree_path, "feat: another feature")

    # Run remove, piping 'n' to abort
    run_workmux_remove(
        env, workmux_exe_path, repo_path, branch_name, force=False, user_input="n"
    )

    assert worktree_path.exists(), "Worktree should NOT be removed after aborting"
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_name in list_windows_result.stdout
    branch_list_result = env.run_command(["git", "branch", "--list", branch_name])
    assert branch_name in branch_list_result.stdout


def test_remove_fails_on_uncommitted_changes(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove` fails if the worktree has uncommitted changes."""
    env = isolated_tmux_server
    branch_name = "dirty-worktree"
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_dirty_file(worktree_path)

    # This should fail because of uncommitted changes
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        force=False,
        expect_fail=True,
    )

    assert worktree_path.exists(), "Worktree should not be removed when command fails"


def test_remove_with_force_on_unmerged_branch(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove -f` removes an unmerged branch without a prompt."""
    env = isolated_tmux_server
    branch_name = "force-remove-unmerged"
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_commit(env, worktree_path, "feat: something unmerged")

    # Force remove should succeed without interaction
    run_workmux_remove(env, workmux_exe_path, repo_path, branch_name, force=True)

    assert not worktree_path.exists(), "Worktree should be removed"


def test_remove_with_force_on_uncommitted_changes(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove -f` removes a worktree with uncommitted changes."""
    env = isolated_tmux_server
    branch_name = "force-remove-dirty"
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_dirty_file(worktree_path)

    # Force remove should succeed despite uncommitted changes
    run_workmux_remove(env, workmux_exe_path, repo_path, branch_name, force=True)

    assert not worktree_path.exists(), "Worktree should be removed"


def test_remove_from_within_worktree_window_without_branch_arg(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove` without branch arg works from within worktree window."""
    env = isolated_tmux_server
    branch_name = "remove-from-within"
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_commit(env, worktree_path, "feat: work to remove")

    # Run remove from within the worktree window without specifying branch name
    # Should auto-detect the current branch and remove it after confirmation
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=None,  # Don't specify branch - should auto-detect
        force=False,
        user_input="y",
        from_window=window_name,
    )

    assert not worktree_path.exists(), "Worktree should be removed"
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_name not in list_windows_result.stdout, "Window should be closed"
    branch_list_result = env.run_command(["git", "branch", "--list", branch_name])
    assert branch_name not in branch_list_result.stdout, "Branch should be removed"


def test_remove_force_from_within_worktree_window_without_branch_arg(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove -f` without branch arg works from within worktree window."""
    env = isolated_tmux_server
    branch_name = "force-remove-from-within"
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_commit(env, worktree_path, "feat: unmerged work")

    # Run remove -f from within the worktree window without specifying branch name
    # Should auto-detect the current branch and remove it without confirmation
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=None,  # Don't specify branch - should auto-detect
        force=True,
        from_window=window_name,
    )

    assert not worktree_path.exists(), "Worktree should be removed"
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_name not in list_windows_result.stdout, "Window should be closed"
    branch_list_result = env.run_command(["git", "branch", "--list", branch_name])
    assert branch_name not in branch_list_result.stdout, "Branch should be removed"


def test_remove_with_keep_branch_flag(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove --keep-branch` removes worktree and window but keeps the branch."""
    env = isolated_tmux_server
    branch_name = "keep-branch-test"
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    create_commit(env, worktree_path, "feat: work to keep")

    # Run remove with --keep-branch flag
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        keep_branch=True,
    )

    # Verify worktree is removed
    assert not worktree_path.exists(), "Worktree should be removed"

    # Verify tmux window is removed
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_name not in list_windows_result.stdout, "Window should be closed"

    # Verify branch still exists
    branch_list_result = env.run_command(["git", "branch", "--list", branch_name])
    assert branch_name in branch_list_result.stdout, "Branch should still exist"


def test_remove_checks_against_stored_base_branch(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies that remove checks for unmerged changes against the stored base branch, not main."""
    env = isolated_tmux_server
    # Use unique branch names to avoid collisions in parallel test execution
    unique_id = uuid.uuid4().hex[:8]
    parent_branch = f"stored-base-parent-{unique_id}"
    child_branch = f"stored-base-child-{unique_id}"
    write_workmux_config(repo_path)

    # Create parent branch from main
    run_workmux_add(env, workmux_exe_path, repo_path, parent_branch)
    parent_worktree = get_worktree_path(repo_path, parent_branch)
    create_commit(env, parent_worktree, "feat: parent work")

    # Create child branch from parent using --base
    run_workmux_add(
        env,
        workmux_exe_path,
        repo_path,
        child_branch,
        base=parent_branch,
        background=True,
    )

    child_worktree = get_worktree_path(repo_path, child_branch)
    create_commit(env, child_worktree, "feat: child work")

    # Verify the base branch was stored in git config
    config_result = env.run_command(
        ["git", "config", "--local", f"branch.{child_branch}.workmux-base"],
        cwd=repo_path,
    )
    assert config_result.returncode == 0, "Base branch should be stored in git config"
    assert parent_branch in config_result.stdout, (
        f"Stored base should be '{parent_branch}', got: {config_result.stdout}"
    )

    # Try to remove child branch - should prompt because it has commits not merged into parent
    # (even though parent itself might not be merged into main)
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        child_branch,
        force=False,
        user_input="n",  # Abort to verify the prompt appears
    )

    # Verify worktree still exists (removal was aborted)
    assert child_worktree.exists(), "Worktree should still exist after aborting"

    # Now confirm the removal
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        child_branch,
        force=False,
        user_input="y",  # Confirm removal
    )

    # Verify child branch was removed
    assert not child_worktree.exists(), "Child worktree should be removed"
    branch_list_result = env.run_command(["git", "branch", "--list", child_branch])
    assert child_branch not in branch_list_result.stdout, (
        "Child branch should be deleted"
    )

    # Parent should still exist
    assert parent_worktree.exists(), "Parent worktree should still exist"


def test_remove_closes_window_with_basename_naming_config(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """
    Verifies that `workmux rm` correctly closes the tmux window when the worktree
    was created with a naming config that differs from the raw branch name.

    This is a lifecycle test that catches bugs where `add` and `rm` derive the
    window name inconsistently. See: the bug where rm used raw branch_name instead
    of the handle derived from the worktree directory basename.
    """
    env = isolated_tmux_server

    # Branch name with a prefix that will be stripped by basename strategy
    branch_name = "feature/TICKET-123-fix-bug"
    # With basename, only "TICKET-123-fix-bug" is used, then slugified
    expected_handle = "ticket-123-fix-bug"
    expected_window = f"{DEFAULT_WINDOW_PREFIX}{expected_handle}"

    # Configure basename naming strategy
    write_workmux_config(repo_path, worktree_naming="basename")

    # Create the worktree
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Verify worktree exists with the derived handle (not the full branch name)
    worktree_parent = repo_path.parent / f"{repo_path.name}__worktrees"
    worktree_path = worktree_parent / expected_handle
    assert worktree_path.is_dir(), (
        f"Worktree should exist at {worktree_path}, "
        f"found: {list(worktree_parent.iterdir()) if worktree_parent.exists() else 'parent not found'}"
    )

    # Verify window exists with the derived name
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert expected_window in list_windows_result.stdout, (
        f"Window {expected_window!r} should exist. "
        f"Found: {list_windows_result.stdout.strip()}"
    )

    # Remove the worktree using the handle (worktree directory name)
    run_workmux_remove(env, workmux_exe_path, repo_path, expected_handle, force=True)

    # Verify worktree is gone
    assert not worktree_path.exists(), "Worktree should be removed"

    # Verify window is closed - this is the key assertion that catches the bug
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert expected_window not in list_windows_result.stdout, (
        f"Window {expected_window!r} should be closed after rm. "
        f"Still found: {list_windows_result.stdout.strip()}"
    )


def test_remove_gone_flag(
    isolated_tmux_server: TmuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    remote_repo_path: Path,
):
    """Verifies `workmux remove --gone` removes worktrees whose upstream was deleted."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    # Add remote to the repo
    env.run_command(
        ["git", "remote", "add", "origin", str(remote_repo_path)], cwd=repo_path
    )

    # 1. Setup a branch with upstream that will be deleted (simulating merged PR)
    gone_branch = "gone-branch"
    window_gone = get_window_name(gone_branch)
    run_workmux_add(env, workmux_exe_path, repo_path, gone_branch)
    gone_worktree = get_worktree_path(repo_path, gone_branch)
    create_commit(env, gone_worktree, "feat: gone work")

    # Push to remote and set upstream
    env.run_command(["git", "push", "-u", "origin", gone_branch], cwd=gone_worktree)

    # Delete the remote branch (simulating what happens after PR merge on GitHub)
    env.run_command(["git", "branch", "-D", gone_branch], cwd=remote_repo_path)

    # 2. Setup a branch with upstream that still exists
    active_branch = "active-branch"
    window_active = get_window_name(active_branch)
    run_workmux_add(env, workmux_exe_path, repo_path, active_branch)
    active_worktree = get_worktree_path(repo_path, active_branch)
    create_commit(env, active_worktree, "feat: active work")

    # Push to remote and set upstream (but don't delete it)
    env.run_command(["git", "push", "-u", "origin", active_branch], cwd=active_worktree)

    # 3. Setup a branch without upstream (local only)
    local_branch = "local-branch"
    window_local = get_window_name(local_branch)
    run_workmux_add(env, workmux_exe_path, repo_path, local_branch)
    local_worktree = get_worktree_path(repo_path, local_branch)

    # Verify all worktrees exist before removal
    assert gone_worktree.exists(), "Gone worktree should exist"
    assert active_worktree.exists(), "Active worktree should exist"
    assert local_worktree.exists(), "Local worktree should exist"

    # 4. Run remove --gone with 'y' confirmation
    # (--gone runs git fetch --prune internally)
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=None,
        gone=True,
        user_input="y",
    )

    # 5. Verify only the "gone" worktree was removed
    assert not gone_worktree.exists(), "Gone branch worktree should be removed"
    assert active_worktree.exists(), "Active branch worktree should remain"
    assert local_worktree.exists(), "Local branch worktree should remain"

    # Verify windows
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window_gone not in list_windows_result.stdout, "Gone window should be closed"
    assert window_active in list_windows_result.stdout, "Active window should remain"
    assert window_local in list_windows_result.stdout, "Local window should remain"

    # Verify branches
    gone_result = env.run_command(
        ["git", "branch", "--list", gone_branch], cwd=repo_path
    )
    assert gone_branch not in gone_result.stdout, "Gone branch should be deleted"

    active_result = env.run_command(
        ["git", "branch", "--list", active_branch], cwd=repo_path
    )
    assert active_branch in active_result.stdout, "Active branch should remain"

    local_result = env.run_command(
        ["git", "branch", "--list", local_branch], cwd=repo_path
    )
    assert local_branch in local_result.stdout, "Local branch should remain"


def test_remove_all_flag(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove --all` removes all worktrees except the main branch."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    # Create multiple worktrees
    branch1 = "feature-one"
    branch2 = "feature-two"
    branch3 = "feature-three"

    window1 = get_window_name(branch1)
    window2 = get_window_name(branch2)
    window3 = get_window_name(branch3)

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)
    run_workmux_add(env, workmux_exe_path, repo_path, branch3)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)
    worktree3 = get_worktree_path(repo_path, branch3)

    # Verify all worktrees exist
    assert worktree1.exists(), "Worktree 1 should exist"
    assert worktree2.exists(), "Worktree 2 should exist"
    assert worktree3.exists(), "Worktree 3 should exist"

    # Run remove --all with confirmation
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        all=True,
        user_input="y",
    )

    # Verify all worktrees were removed
    assert not worktree1.exists(), "Worktree 1 should be removed"
    assert not worktree2.exists(), "Worktree 2 should be removed"
    assert not worktree3.exists(), "Worktree 3 should be removed"

    # Verify windows are closed
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window1 not in list_windows_result.stdout, "Window 1 should be closed"
    assert window2 not in list_windows_result.stdout, "Window 2 should be closed"
    assert window3 not in list_windows_result.stdout, "Window 3 should be closed"

    # Verify branches are deleted
    for branch in [branch1, branch2, branch3]:
        result = env.run_command(["git", "branch", "--list", branch], cwd=repo_path)
        assert branch not in result.stdout, f"Branch {branch} should be deleted"


def test_remove_all_with_force(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove --all -f` removes all worktrees without confirmation."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "force-all-one"
    branch2 = "force-all-two"

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)

    # Create uncommitted changes in one worktree
    create_dirty_file(worktree1)

    # Create unmerged commits in another worktree
    create_commit(env, worktree2, "feat: unmerged work")

    # Run remove --all --force (should succeed without prompts)
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        all=True,
        force=True,
    )

    # Verify all worktrees were removed
    assert not worktree1.exists(), "Worktree with uncommitted changes should be removed"
    assert not worktree2.exists(), "Worktree with unmerged commits should be removed"


def test_remove_all_skips_unmerged_without_force(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove --all` skips worktrees with unmerged commits unless --force is used."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    # Create a clean branch (no unmerged commits)
    clean_branch = "all-clean-branch"
    run_workmux_add(env, workmux_exe_path, repo_path, clean_branch)
    clean_worktree = get_worktree_path(repo_path, clean_branch)

    # Create a branch with unmerged commits
    unmerged_branch = "all-unmerged-branch"
    run_workmux_add(env, workmux_exe_path, repo_path, unmerged_branch)
    unmerged_worktree = get_worktree_path(repo_path, unmerged_branch)
    create_commit(env, unmerged_worktree, "feat: unmerged work")

    # Run remove --all with confirmation
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        all=True,
        user_input="y",
    )

    # Clean branch should be removed
    assert not clean_worktree.exists(), "Clean worktree should be removed"

    # Unmerged branch should be skipped (still exists)
    assert unmerged_worktree.exists(), "Unmerged worktree should be skipped"

    # Verify the unmerged branch still exists
    result = env.run_command(
        ["git", "branch", "--list", unmerged_branch], cwd=repo_path
    )
    assert unmerged_branch in result.stdout, "Unmerged branch should still exist"


def test_remove_all_with_keep_branch(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove --all --keep-branch` removes worktrees but keeps branches."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "keep-all-one"
    branch2 = "keep-all-two"

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)

    # Add unmerged commits (should not block removal when using --keep-branch)
    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)
    create_commit(env, worktree1, "feat: work one")
    create_commit(env, worktree2, "feat: work two")

    # Run remove --all --keep-branch with confirmation
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        all=True,
        keep_branch=True,
        user_input="y",
    )

    # Verify worktrees were removed
    assert not worktree1.exists(), "Worktree 1 should be removed"
    assert not worktree2.exists(), "Worktree 2 should be removed"

    # Verify branches still exist
    for branch in [branch1, branch2]:
        result = env.run_command(["git", "branch", "--list", branch], cwd=repo_path)
        assert branch in result.stdout, f"Branch {branch} should still exist"


def test_remove_multiple_branches(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove branch1 branch2` removes multiple worktrees at once."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "multi-rm-one"
    branch2 = "multi-rm-two"
    branch3 = "multi-rm-three"

    window1 = get_window_name(branch1)
    window2 = get_window_name(branch2)
    window3 = get_window_name(branch3)

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)
    run_workmux_add(env, workmux_exe_path, repo_path, branch3)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)
    worktree3 = get_worktree_path(repo_path, branch3)

    # Verify all worktrees exist
    assert worktree1.exists(), "Worktree 1 should exist"
    assert worktree2.exists(), "Worktree 2 should exist"
    assert worktree3.exists(), "Worktree 3 should exist"

    # Remove two of the three worktrees by specifying multiple names
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=f"{branch1} {branch2}",
        force=True,
    )

    # Verify first two worktrees were removed
    assert not worktree1.exists(), "Worktree 1 should be removed"
    assert not worktree2.exists(), "Worktree 2 should be removed"

    # Verify third worktree still exists
    assert worktree3.exists(), "Worktree 3 should still exist"

    # Verify windows are closed for removed worktrees
    list_windows_result = env.tmux(["list-windows", "-F", "#{window_name}"])
    assert window1 not in list_windows_result.stdout, "Window 1 should be closed"
    assert window2 not in list_windows_result.stdout, "Window 2 should be closed"
    assert window3 in list_windows_result.stdout, "Window 3 should still exist"

    # Verify branches are deleted for removed worktrees
    for branch in [branch1, branch2]:
        result = env.run_command(["git", "branch", "--list", branch], cwd=repo_path)
        assert branch not in result.stdout, f"Branch {branch} should be deleted"

    # Verify third branch still exists
    result = env.run_command(["git", "branch", "--list", branch3], cwd=repo_path)
    assert branch3 in result.stdout, f"Branch {branch3} should still exist"


def test_remove_multiple_with_unmerged_prompts_once(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove branch1 branch2` prompts once for all unmerged branches."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "multi-unmerged-one"
    branch2 = "multi-unmerged-two"

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)

    # Add unmerged commits to both worktrees
    create_commit(env, worktree1, "feat: unmerged work one")
    create_commit(env, worktree2, "feat: unmerged work two")

    # Remove both - should prompt once and remove both after confirmation
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=f"{branch1} {branch2}",
        user_input="y",
    )

    # Verify both worktrees were removed
    assert not worktree1.exists(), "Worktree 1 should be removed"
    assert not worktree2.exists(), "Worktree 2 should be removed"


def test_remove_multiple_aborted_keeps_all(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies aborting multi-branch removal keeps all worktrees."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "multi-abort-one"
    branch2 = "multi-abort-two"

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)

    # Add unmerged commits
    create_commit(env, worktree1, "feat: unmerged work one")
    create_commit(env, worktree2, "feat: unmerged work two")

    # Abort the removal
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=f"{branch1} {branch2}",
        user_input="n",
    )

    # Verify both worktrees still exist
    assert worktree1.exists(), "Worktree 1 should still exist"
    assert worktree2.exists(), "Worktree 2 should still exist"


def test_remove_multiple_with_uncommitted_fails(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies removing multiple branches fails if any has uncommitted changes."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "multi-dirty-one"
    branch2 = "multi-dirty-two"

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)

    # Add uncommitted changes to one worktree
    create_dirty_file(worktree1)

    # Should fail because of uncommitted changes
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=f"{branch1} {branch2}",
        expect_fail=True,
    )

    # Verify both worktrees still exist (none removed due to atomic behavior)
    assert worktree1.exists(), "Worktree 1 should still exist"
    assert worktree2.exists(), "Worktree 2 should still exist"


def test_remove_multiple_with_keep_branch(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux remove --keep-branch branch1 branch2` removes worktrees but keeps branches."""
    env = isolated_tmux_server
    write_workmux_config(repo_path)

    branch1 = "multi-keep-one"
    branch2 = "multi-keep-two"

    run_workmux_add(env, workmux_exe_path, repo_path, branch1)
    run_workmux_add(env, workmux_exe_path, repo_path, branch2)

    worktree1 = get_worktree_path(repo_path, branch1)
    worktree2 = get_worktree_path(repo_path, branch2)

    # Add unmerged commits (should not prompt because --keep-branch doesn't delete branches)
    create_commit(env, worktree1, "feat: work one")
    create_commit(env, worktree2, "feat: work two")

    # Remove both with --keep-branch (no confirmation needed)
    run_workmux_remove(
        env,
        workmux_exe_path,
        repo_path,
        branch_name=f"{branch1} {branch2}",
        keep_branch=True,
    )

    # Verify worktrees were removed
    assert not worktree1.exists(), "Worktree 1 should be removed"
    assert not worktree2.exists(), "Worktree 2 should be removed"

    # Verify branches still exist
    for branch in [branch1, branch2]:
        result = env.run_command(["git", "branch", "--list", branch], cwd=repo_path)
        assert branch in result.stdout, f"Branch {branch} should still exist"
