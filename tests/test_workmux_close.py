from pathlib import Path

from .conftest import (
    MuxEnvironment,
    WorkmuxCommandResult,
    get_window_name,
    get_worktree_path,
    poll_until,
    run_workmux_add,
    run_workmux_command,
    write_workmux_config,
)


def run_workmux_close(
    env: MuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    name: str | None = None,
    expect_fail: bool = False,
) -> WorkmuxCommandResult:
    """
    Helper to run `workmux close` command.

    Uses run_shell_background to avoid hanging when close kills the current window.
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

    env.run_shell_background(close_script)

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


def test_close_kills_window_keeps_worktree(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `workmux close` kills the multiplexer window but keeps the worktree."""
    env = mux_server
    branch_name = "feature-close-test"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Verify window exists before close
    windows = env.list_windows()
    assert window_name in windows

    # Close the window
    run_workmux_close(env, workmux_exe_path, mux_repo_path, branch_name)

    # Verify window is gone
    windows = env.list_windows()
    assert window_name not in windows

    # Verify worktree still exists
    worktree_path = get_worktree_path(mux_repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after close"


def test_close_fails_when_no_window_exists(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `workmux close` fails if no window exists for the worktree."""
    env = mux_server
    branch_name = "feature-no-window"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Kill the window manually
    env.kill_window(window_name)

    # Now try to close - should fail because window doesn't exist
    result = run_workmux_close(
        env,
        workmux_exe_path,
        mux_repo_path,
        branch_name,
        expect_fail=True,
    )

    assert "No active window found" in result.stderr


def test_close_fails_when_worktree_missing(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `workmux close` fails if the worktree does not exist."""
    env = mux_server
    worktree_name = "nonexistent-worktree"

    write_workmux_config(mux_repo_path)

    result = run_workmux_close(
        env,
        workmux_exe_path,
        mux_repo_path,
        worktree_name,
        expect_fail=True,
    )

    assert "No worktree found with name" in result.stderr


def test_close_can_reopen_with_open(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies that after `workmux close`, `workmux open` can recreate the window."""
    env = mux_server
    branch_name = "feature-close-reopen"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Close the window
    run_workmux_close(env, workmux_exe_path, mux_repo_path, branch_name)

    # Verify window is gone
    windows = env.list_windows()
    assert window_name not in windows

    # Reopen with workmux open
    run_workmux_command(env, workmux_exe_path, mux_repo_path, f"open {branch_name}")

    # Verify window is back
    windows = env.list_windows()
    assert window_name in windows


def test_close_from_inside_worktree_window(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies `workmux close` works when run from inside the target window itself."""
    env = mux_server
    branch_name = "feature-self-close"
    window_name = get_window_name(branch_name)

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Verify window exists
    windows = env.list_windows()
    assert window_name in windows

    # Send keystrokes directly to the worktree window to run close
    # This tests the schedule_window_close path (self-closing)
    cmd = f"{workmux_exe_path} close"
    env.send_keys(window_name, cmd, enter=True)

    # Poll until window is gone
    def window_is_gone():
        return window_name not in env.list_windows()

    assert poll_until(window_is_gone, timeout=5.0), "Window did not close itself"

    # Verify worktree still exists
    worktree_path = get_worktree_path(mux_repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after self-close"
