from pathlib import Path

from .conftest import (
    TmuxEnvironment,
    WorkmuxCommandResult,
    get_window_name,
    get_worktree_path,
    poll_until,
    run_workmux_add,
    run_workmux_command,
    write_workmux_config,
)


def run_workmux_close(
    env: TmuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    name: str | None = None,
    expect_fail: bool = False,
) -> WorkmuxCommandResult:
    """
    Helper to run `workmux close` command.

    Uses tmux run-shell -b to avoid hanging when close kills the current window.
    """
    stdout_file = env.tmp_path / "workmux_close_stdout.txt"
    stderr_file = env.tmp_path / "workmux_close_stderr.txt"
    exit_code_file = env.tmp_path / "workmux_close_exit_code.txt"

    for f in [stdout_file, stderr_file, exit_code_file]:
        if f.exists():
            f.unlink()

    name_arg = name if name else ""
    close_script = (
        f"cd {repo_path} && "
        f"{workmux_exe_path} close {name_arg} "
        f"> {stdout_file} 2> {stderr_file}; "
        f"echo $? > {exit_code_file}"
    )

    env.tmux(["run-shell", "-b", close_script])

    assert poll_until(exit_code_file.exists, timeout=5.0), (
        "workmux close did not complete in time"
    )

    exit_code = int(exit_code_file.read_text().strip())
    stderr = stderr_file.read_text() if stderr_file.exists() else ""
    stdout = stdout_file.read_text() if stdout_file.exists() else ""

    result = WorkmuxCommandResult(
        exit_code=exit_code,
        stdout=stdout,
        stderr=stderr,
    )

    if expect_fail:
        if exit_code == 0:
            raise AssertionError(
                f"workmux close was expected to fail but succeeded.\nStderr:\n{stderr}"
            )
    else:
        if exit_code != 0:
            raise AssertionError(
                f"workmux close failed with exit code {exit_code}\nStderr:\n{stderr}"
            )

    return result


def test_close_kills_tmux_window_keeps_worktree(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux close` kills the tmux window but keeps the worktree."""
    env = isolated_tmux_server
    branch_name = "feature-close-test"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Verify window exists before close
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows

    # Close the window
    run_workmux_close(env, workmux_exe_path, repo_path, branch_name)

    # Verify window is gone
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name not in list_windows

    # Verify worktree still exists
    worktree_path = get_worktree_path(repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after close"


def test_close_fails_when_no_window_exists(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux close` fails if no tmux window exists for the worktree."""
    env = isolated_tmux_server
    branch_name = "feature-no-window"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Kill the window manually
    env.tmux(["kill-window", "-t", window_name])

    # Now try to close - should fail because window doesn't exist
    result = run_workmux_close(
        env,
        workmux_exe_path,
        repo_path,
        branch_name,
        expect_fail=True,
    )

    assert "No active tmux window found" in result.stderr


def test_close_fails_when_worktree_missing(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux close` fails if the worktree does not exist."""
    env = isolated_tmux_server
    worktree_name = "nonexistent-worktree"

    write_workmux_config(repo_path)

    result = run_workmux_close(
        env,
        workmux_exe_path,
        repo_path,
        worktree_name,
        expect_fail=True,
    )

    assert "No worktree found with name" in result.stderr


def test_close_can_reopen_with_open(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies that after `workmux close`, `workmux open` can recreate the window."""
    env = isolated_tmux_server
    branch_name = "feature-close-reopen"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Close the window
    run_workmux_close(env, workmux_exe_path, repo_path, branch_name)

    # Verify window is gone
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name not in list_windows

    # Reopen with workmux open
    run_workmux_command(env, workmux_exe_path, repo_path, f"open {branch_name}")

    # Verify window is back
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows


def test_close_from_inside_worktree_window(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """Verifies `workmux close` works when run from inside the target window itself."""
    env = isolated_tmux_server
    branch_name = "feature-self-close"
    window_name = get_window_name(branch_name)

    write_workmux_config(repo_path)
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Verify window exists
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows

    # Send keystrokes directly to the worktree window to run close
    # This tests the schedule_window_close path (self-closing)
    cmd = f"{workmux_exe_path} close"
    env.tmux(["send-keys", "-t", window_name, cmd, "Enter"])

    # Poll until window is gone
    def window_is_gone():
        windows = env.tmux(
            ["list-windows", "-F", "#{window_name}"]
        ).stdout.splitlines()
        return window_name not in windows

    assert poll_until(window_is_gone, timeout=5.0), "Window did not close itself"

    # Verify worktree still exists
    worktree_path = get_worktree_path(repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after self-close"
