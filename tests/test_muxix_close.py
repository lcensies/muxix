from pathlib import Path

import pytest

from .conftest import (
    DEFAULT_WINDOW_PREFIX,
    MuxEnvironment,
    MuxixCommandResult,
    TmuxEnvironment,
    get_scripts_dir,
    get_window_name,
    get_worktree_path,
    poll_until,
    poll_until_file_has_content,
    run_muxix_add,
    run_muxix_command,
    write_muxix_config,
)


def run_muxix_close(
    env: MuxEnvironment,
    muxix_exe_path: Path,
    repo_path: Path,
    name: str | None = None,
    expect_fail: bool = False,
) -> MuxixCommandResult:
    """
    Helper to run `muxix close` command.

    Uses run_shell_background to avoid hanging when close kills the current window.
    """
    scripts_dir = get_scripts_dir(env)
    stdout_file = scripts_dir / "muxix_close_stdout.txt"
    stderr_file = scripts_dir / "muxix_close_stderr.txt"
    exit_code_file = scripts_dir / "muxix_close_exit_code.txt"

    for f in [stdout_file, stderr_file, exit_code_file]:
        if f.exists():
            f.unlink()

    name_arg = name if name else ""
    close_script = (
        f"cd {repo_path} && "
        f"{muxix_exe_path} close {name_arg} "
        f"> {stdout_file} 2> {stderr_file}; "
        f"echo $? > {exit_code_file}"
    )

    env.run_shell_background(close_script)

    assert poll_until_file_has_content(exit_code_file, timeout=5.0), (
        "muxix close did not complete in time"
    )

    exit_code = int(exit_code_file.read_text().strip())
    stderr = stderr_file.read_text() if stderr_file.exists() else ""
    stdout = stdout_file.read_text() if stdout_file.exists() else ""

    result = MuxixCommandResult(
        exit_code=exit_code,
        stdout=stdout,
        stderr=stderr,
    )

    if expect_fail:
        if exit_code == 0:
            raise AssertionError(
                f"muxix close was expected to fail but succeeded.\nStderr:\n{stderr}"
            )
    else:
        if exit_code != 0:
            raise AssertionError(
                f"muxix close failed with exit code {exit_code}\nStderr:\n{stderr}"
            )

    return result


def test_close_kills_window_keeps_worktree(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix close` kills the multiplexer window but keeps the worktree."""
    env = mux_server
    branch_name = "feature-close-test"
    window_name = get_window_name(branch_name)

    write_muxix_config(mux_repo_path)
    run_muxix_add(env, muxix_exe_path, mux_repo_path, branch_name)

    # Verify window exists before close
    windows = env.list_windows()
    assert window_name in windows

    # Close the window
    run_muxix_close(env, muxix_exe_path, mux_repo_path, branch_name)

    # Verify window is gone
    windows = env.list_windows()
    assert window_name not in windows

    # Verify worktree still exists
    worktree_path = get_worktree_path(mux_repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after close"


@pytest.mark.tmux_only
def test_close_window_in_parent_session(
    mux_server: TmuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    env = mux_server
    branch_name = "feature/parent-session-close"
    session_name = "prs-close"
    window_name = get_window_name(branch_name)

    write_muxix_config(mux_repo_path)
    run_muxix_command(
        env,
        muxix_exe_path,
        mux_repo_path,
        f"add {branch_name} --parent-session {session_name} --background",
    )

    result = env.tmux(
        ["list-windows", "-t", f"{session_name}:", "-F", "#{window_name}"]
    )
    assert window_name in [w for w in result.stdout.strip().split("\n") if w]

    run_muxix_close(env, muxix_exe_path, mux_repo_path, branch_name)

    result = env.tmux(
        ["list-windows", "-t", f"{session_name}:", "-F", "#{window_name}"],
        check=False,
    )
    windows = [w for w in result.stdout.strip().split("\n") if w]
    assert result.returncode != 0 or window_name not in windows
    worktree_path = get_worktree_path(mux_repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after close"


def test_close_fails_when_no_window_exists(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix close` fails if no window exists for the worktree."""
    env = mux_server
    branch_name = "feature-no-window"
    window_name = get_window_name(branch_name)

    write_muxix_config(mux_repo_path)
    run_muxix_add(env, muxix_exe_path, mux_repo_path, branch_name)

    # Kill the window manually
    env.kill_window(window_name)

    # Now try to close - should fail because window doesn't exist
    result = run_muxix_close(
        env,
        muxix_exe_path,
        mux_repo_path,
        branch_name,
        expect_fail=True,
    )

    assert "No active window found" in result.stderr


def test_close_fails_when_worktree_missing(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix close` fails if the worktree does not exist."""
    env = mux_server
    worktree_name = "nonexistent-worktree"

    write_muxix_config(mux_repo_path)

    result = run_muxix_close(
        env,
        muxix_exe_path,
        mux_repo_path,
        worktree_name,
        expect_fail=True,
    )

    assert "not found" in result.stderr


def test_close_can_reopen_with_open(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies that after `muxix close`, `muxix open` can recreate the window."""
    env = mux_server
    branch_name = "feature-close-reopen"
    window_name = get_window_name(branch_name)

    write_muxix_config(mux_repo_path)
    run_muxix_add(env, muxix_exe_path, mux_repo_path, branch_name)

    # Close the window
    run_muxix_close(env, muxix_exe_path, mux_repo_path, branch_name)

    # Verify window is gone
    windows = env.list_windows()
    assert window_name not in windows

    # Reopen with muxix open
    run_muxix_command(env, muxix_exe_path, mux_repo_path, f"open {branch_name}")

    # Verify window is back
    windows = env.list_windows()
    assert window_name in windows


def test_close_from_inside_worktree_window(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix close` works when run from inside the target window itself."""
    env = mux_server
    branch_name = "feature-self-close"
    window_name = get_window_name(branch_name)

    write_muxix_config(mux_repo_path)
    run_muxix_add(env, muxix_exe_path, mux_repo_path, branch_name)

    # Verify window exists
    windows = env.list_windows()
    assert window_name in windows

    # Send keystrokes directly to the worktree window to run close
    # This tests the schedule_window_close path (self-closing)
    cmd = f"{muxix_exe_path} close"
    env.send_keys(window_name, cmd, enter=True)

    # Poll until window is gone
    def window_is_gone():
        return window_name not in env.list_windows()

    assert poll_until(window_is_gone, timeout=5.0), "Window did not close itself"

    # Verify worktree still exists
    worktree_path = get_worktree_path(mux_repo_path, branch_name)
    assert worktree_path.exists(), "Worktree should still exist after self-close"


def test_close_by_branch_name_when_handle_differs(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix close <branch>` resolves correctly when handle differs from branch name."""
    env = mux_server
    branch_name = "feature/close-handle-test"
    handle = "close-handle-test"
    window_name = f"{DEFAULT_WINDOW_PREFIX}{handle}"

    write_muxix_config(mux_repo_path)
    run_muxix_command(
        env,
        muxix_exe_path,
        mux_repo_path,
        f"add {branch_name} --name {handle}",
    )

    # Verify window exists (named after handle, not branch)
    windows = env.list_windows()
    assert window_name in windows

    # Close using the branch name -- this should resolve to the correct handle
    run_muxix_close(env, muxix_exe_path, mux_repo_path, branch_name)

    # Verify window is gone
    windows = env.list_windows()
    assert window_name not in windows
