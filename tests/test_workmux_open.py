from pathlib import Path

from .conftest import (
    TmuxEnvironment,
    get_window_name,
    get_worktree_path,
    run_workmux_add,
    run_workmux_open,
    write_workmux_config,
)


def _kill_window(env: TmuxEnvironment, branch_name: str) -> None:
    """Helper to close the tmux window for a branch if it exists."""
    window_name = get_window_name(branch_name)
    env.tmux(["has-session", "-t", window_name], check=False)
    env.tmux(["kill-window", "-t", window_name], check=False)


def test_open_recreates_tmux_window_for_existing_worktree(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` recreates a tmux window for an existing worktree."""
    env = isolated_tmux_server
    branch_name = "feature-open-success"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Close the original window to simulate a detached worktree
    env.tmux(["kill-window", "-t", window_name])

    run_workmux_open(env, workmux_exe_path, repo_path, branch_name)

    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows


def test_open_fails_when_window_already_exists(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` fails when the tmux window already exists."""
    env = isolated_tmux_server
    branch_name = "feature-open-window-exists"

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    result = run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        expect_fail=True,
    )

    assert "window named" in result.stderr


def test_open_fails_when_worktree_missing(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open` fails if the worktree does not exist."""
    env = isolated_tmux_server
    worktree_name = "missing-worktree"

    write_workmux_config(repo_path)

    result = run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        worktree_name,
        expect_fail=True,
    )

    assert "No worktree found with name" in result.stderr


def test_open_with_run_hooks_reexecutes_post_create_commands(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --run-hooks` re-runs post_create hooks."""
    env = isolated_tmux_server
    branch_name = "feature-open-hooks"
    hook_file = "open_hook.txt"

    write_workmux_config(repo_path, post_create=[f"touch {hook_file}"])
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    hook_path = worktree_path / hook_file
    hook_path.unlink()

    _kill_window(env, branch_name)

    run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        run_hooks=True,
    )

    assert hook_path.exists()


def test_open_with_force_files_reapplies_file_operations(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux open --force-files` reapplies copy operations."""
    env = isolated_tmux_server
    branch_name = "feature-open-files"
    shared_file = repo_path / "shared.env"
    shared_file.write_text("KEY=value")

    write_workmux_config(repo_path, files={"copy": ["shared.env"]})
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    worktree_path = get_worktree_path(repo_path, branch_name)
    worktree_file = worktree_path / "shared.env"
    worktree_file.unlink()

    _kill_window(env, branch_name)

    run_workmux_open(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        force_files=True,
    )

    assert worktree_file.exists()
    assert worktree_file.read_text() == "KEY=value"
