"""
Tests for PR checkout functionality (workmux add --pr <number>)
"""

from pathlib import Path

from .conftest import (
    TmuxEnvironment,
    get_window_name,
    get_worktree_path,
    install_fake_gh_cli,
    run_workmux_command,
    setup_git_repo,
)


def setup_pr_remote_and_branch(
    env: TmuxEnvironment,
    repo_path: Path,
    remote_repo_path: Path,
    branch_name: str,
):
    """Helper to set up a fetchable remote with a PR branch"""
    # Use a fake GitHub URL for the remote so get_repo_owner() can parse it
    github_url = "https://github.com/testowner/testrepo.git"

    env.run_command(
        ["git", "remote", "add", "origin", github_url],
        cwd=repo_path,
    )
    # Set pushurl to the local path so git operations actually work
    env.run_command(
        ["git", "remote", "set-url", "--push", "origin", str(remote_repo_path)],
        cwd=repo_path,
    )
    # Also need to configure insteadOf for fetch operations
    env.run_command(
        ["git", "config", f"url.{remote_repo_path}.insteadOf", github_url],
        cwd=repo_path,
    )
    env.run_command(["git", "push", "-u", "origin", "main"], cwd=repo_path)

    # Create and push the PR branch
    env.run_command(["git", "checkout", "-b", branch_name], cwd=repo_path)
    env.run_command(
        ["git", "commit", "--allow-empty", "-m", "PR changes"],
        cwd=repo_path,
    )
    env.run_command(["git", "push", "-u", "origin", branch_name], cwd=repo_path)
    env.run_command(["git", "checkout", "main"], cwd=repo_path)
    # Delete the local branch so workmux can create it fresh (matching gh pr checkout behavior)
    env.run_command(["git", "branch", "-D", branch_name], cwd=repo_path)


def test_add_pr_from_same_repo(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test basic PR checkout from same repository"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    setup_pr_remote_and_branch(env, repo_path, remote_repo_path, "feature-branch")

    pr_data = {
        "headRefName": "feature-branch",
        "headRepositoryOwner": {"login": "testowner"},
        "state": "OPEN",
        "isDraft": False,
        "title": "Add new feature",
        "author": {"login": "contributor"},
    }
    install_fake_gh_cli(env, pr_number=123, json_response=pr_data)

    result = run_workmux_command(env, workmux_exe_path, repo_path, "add --pr 123")

    assert "PR #123" in result.stdout
    assert "Add new feature" in result.stdout
    assert "contributor" in result.stdout

    worktree_path = get_worktree_path(repo_path, "feature-branch")
    assert worktree_path.exists()

    window_name = get_window_name("feature-branch")
    windows_result = env.tmux(["list-windows", "-t", "test"], check=False)
    assert window_name in windows_result.stdout


def test_add_pr_with_custom_branch_name(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test PR checkout with custom branch name"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    setup_pr_remote_and_branch(env, repo_path, remote_repo_path, "feature-branch")

    pr_data = {
        "headRefName": "feature-branch",
        "headRepositoryOwner": {"login": "testowner"},
        "state": "OPEN",
        "isDraft": False,
        "title": "Add new feature",
        "author": {"login": "contributor"},
    }
    install_fake_gh_cli(env, pr_number=123, json_response=pr_data)

    result = run_workmux_command(
        env, workmux_exe_path, repo_path, "add my-review --pr 123"
    )

    assert "PR #123" in result.stdout

    worktree_path = get_worktree_path(repo_path, "my-review")
    assert worktree_path.exists()

    window_name = get_window_name("my-review")
    windows_result = env.tmux(["list-windows", "-t", "test"], check=False)
    assert window_name in windows_result.stdout


def test_add_pr_merged_state_warning(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test warning is displayed for merged PRs"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    setup_pr_remote_and_branch(env, repo_path, remote_repo_path, "merged-branch")

    pr_data = {
        "headRefName": "merged-branch",
        "headRepositoryOwner": {"login": "testowner"},
        "state": "MERGED",
        "isDraft": False,
        "title": "Already merged PR",
        "author": {"login": "contributor"},
    }
    install_fake_gh_cli(env, pr_number=456, json_response=pr_data)

    result = run_workmux_command(env, workmux_exe_path, repo_path, "add --pr 456")

    assert "Warning" in result.stderr or "MERGED" in result.stderr
    assert "456" in result.stdout

    worktree_path = get_worktree_path(repo_path, "merged-branch")
    assert worktree_path.exists()


def test_add_pr_draft_warning(isolated_tmux_server, workmux_exe_path, remote_repo_path):
    """Test warning is displayed for draft PRs"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    setup_pr_remote_and_branch(env, repo_path, remote_repo_path, "draft-branch")

    pr_data = {
        "headRefName": "draft-branch",
        "headRepositoryOwner": {"login": "testowner"},
        "state": "OPEN",
        "isDraft": True,
        "title": "WIP: Work in progress",
        "author": {"login": "contributor"},
    }
    install_fake_gh_cli(env, pr_number=789, json_response=pr_data)

    result = run_workmux_command(env, workmux_exe_path, repo_path, "add --pr 789")

    assert "DRAFT" in result.stderr or "draft" in result.stderr.lower()

    worktree_path = get_worktree_path(repo_path, "draft-branch")
    assert worktree_path.exists()


def test_add_pr_fails_on_invalid_pr_number(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test error handling for invalid PR number"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    env.run_command(
        ["git", "remote", "add", "origin", str(remote_repo_path)],
        cwd=repo_path,
    )

    install_fake_gh_cli(
        env,
        pr_number=999,
        json_response=None,
        stderr="pull request not found",
        exit_code=1,
    )

    result = run_workmux_command(
        env, workmux_exe_path, repo_path, "add --pr 999", expect_fail=True
    )

    assert result.exit_code != 0
    assert (
        "Failed to fetch" in result.stderr or "pull request not found" in result.stderr
    )


def test_add_pr_fails_when_gh_not_installed(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test error when gh CLI is not available"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    env.run_command(
        ["git", "remote", "add", "origin", str(remote_repo_path)],
        cwd=repo_path,
    )

    # Don't install fake gh CLI - it won't be found in PATH

    result = run_workmux_command(
        env, workmux_exe_path, repo_path, "add --pr 123", expect_fail=True
    )

    assert result.exit_code != 0
    assert "gh" in result.stderr.lower() or "GitHub CLI" in result.stderr


def test_add_pr_conflicts_with_base_flag(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test that --pr conflicts with --base flag"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    result = run_workmux_command(
        env,
        workmux_exe_path,
        repo_path,
        "add --pr 123 --base main",
        expect_fail=True,
    )

    assert result.exit_code != 0
    assert (
        "conflict" in result.stderr.lower() or "cannot be used" in result.stderr.lower()
    )


def test_add_pr_fails_when_worktree_exists(
    isolated_tmux_server, workmux_exe_path, remote_repo_path
):
    """Test error when trying to checkout same PR twice"""
    env = isolated_tmux_server
    repo_path = env.tmp_path
    setup_git_repo(repo_path, env.env)

    setup_pr_remote_and_branch(env, repo_path, remote_repo_path, "feature-branch")

    pr_data = {
        "headRefName": "feature-branch",
        "headRepositoryOwner": {"login": "testowner"},
        "state": "OPEN",
        "isDraft": False,
        "title": "Add new feature",
        "author": {"login": "contributor"},
    }
    install_fake_gh_cli(env, pr_number=123, json_response=pr_data)

    # First checkout should succeed
    run_workmux_command(env, workmux_exe_path, repo_path, "add --pr 123")

    # Second checkout should fail
    result = run_workmux_command(
        env, workmux_exe_path, repo_path, "add --pr 123", expect_fail=True
    )

    assert result.exit_code != 0
    assert (
        "already exists" in result.stderr.lower() or "worktree" in result.stderr.lower()
    )
