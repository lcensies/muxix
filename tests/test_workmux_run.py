"""
Tests for `workmux run` command.

Covers behavior unique to the two-process `run` architecture (coordinator
splits a pane running `_exec`, streams output via files, propagates exit codes).
Error paths shared with other coordinator commands (worktree not found, no agent)
are already tested in test_workmux_send/capture/wait.
"""

import json
import re
import shlex
import time
from pathlib import Path

from .conftest import (
    MuxEnvironment,
    WorkmuxCommandResult,
    get_window_name,
    poll_until,
    poll_until_file_has_content,
    run_workmux_add,
    write_workmux_config,
)

from .test_agent_state import build_status_cmd, list_agent_state_files


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def extract_artifacts_path(stderr: str) -> Path:
    """Parse the artifact directory path emitted by `workmux run` on stderr."""
    m = re.search(r"Artifacts(?:\s+kept\s+at)?:\s*(.+)", stderr)
    assert m, f"Could not find artifacts path in stderr:\n{stderr}"
    return Path(m.group(1).strip())


def setup_worktree_with_agent(
    env: MuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    branch_name: str,
) -> None:
    """Create a worktree and establish agent state so coordinator commands work."""
    window_name = get_window_name(branch_name)
    write_workmux_config(repo_path, panes=[{"focus": True}])
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)
    time.sleep(1.5)

    status_cmd = build_status_cmd(env, workmux_exe_path, "waiting")
    env.send_keys(window_name, status_cmd)
    assert poll_until(lambda: len(list_agent_state_files(env)) > 0, timeout=5.0), (
        "Agent state file not created"
    )

    # Ensure the shell has returned to its prompt before the next command.
    # poll_until returns as soon as the state file exists, but the shell may
    # still be finishing set-window-status output. Touch a marker file so we
    # can poll for prompt readiness without a fixed sleep.
    marker = env.tmp_path / f"ready-{branch_name}"
    env.send_keys(window_name, f"touch {shlex.quote(str(marker))}")
    assert poll_until(lambda: marker.exists(), timeout=3.0), (
        "Shell did not return to prompt after set-window-status"
    )


def run_workmux_run(
    env: MuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    command: str,
    expect_fail: bool = False,
) -> WorkmuxCommandResult:
    """Run a `workmux run` command with extended timeout.

    The `run` command involves splitting a pane, starting the `_exec` process,
    and streaming output -- more latency than simple coordinator commands.
    Uses a 10s timeout instead of the default 5s.
    """
    stdout_file = env.tmp_path / "workmux_run_stdout.txt"
    stderr_file = env.tmp_path / "workmux_run_stderr.txt"
    exit_code_file = env.tmp_path / "workmux_run_exit_code.txt"

    for f in [stdout_file, stderr_file, exit_code_file]:
        if f.exists():
            f.unlink()

    workdir_str = shlex.quote(str(repo_path))
    exe_str = shlex.quote(str(workmux_exe_path))
    stdout_str = shlex.quote(str(stdout_file))
    stderr_str = shlex.quote(str(stderr_file))
    exit_code_str = shlex.quote(str(exit_code_file))

    env_vars = [f"PATH={shlex.quote(env.env['PATH'])}"]
    if "TMPDIR" in env.env:
        env_vars.append(f"TMPDIR={shlex.quote(env.env['TMPDIR'])}")
    if "HOME" in env.env:
        env_vars.append(f"HOME={shlex.quote(env.env['HOME'])}")
    env_str = " ".join(env_vars)

    workmux_cmd = (
        f"cd {workdir_str} && "
        f"env {env_str} WORKMUX_TEST=1 {exe_str} {command} "
        f"> {stdout_str} 2> {stderr_str}; "
        f"echo $? > {exit_code_str}"
    )

    env.send_keys("test:", workmux_cmd, enter=True)

    if not poll_until_file_has_content(exit_code_file, timeout=10.0):
        pane_content = env.capture_pane("test") or "(empty)"
        raise AssertionError(
            f"workmux run did not complete in time\nPane content:\n{pane_content}"
        )

    result = WorkmuxCommandResult(
        exit_code=int(exit_code_file.read_text().strip()),
        stdout=stdout_file.read_text() if stdout_file.exists() else "",
        stderr=stderr_file.read_text() if stderr_file.exists() else "",
    )

    if expect_fail:
        if result.exit_code == 0:
            raise AssertionError(
                f"workmux {command} was expected to fail but succeeded.\n"
                f"Stdout:\n{result.stdout}"
            )
    else:
        if result.exit_code != 0:
            raise AssertionError(
                f"workmux {command} failed with exit code {result.exit_code}\n"
                f"{result.stderr}"
            )

    return result


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_run_streams_stdout_and_stderr(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Run forwards the child's stdout and stderr via file-based IPC."""
    env = mux_server
    setup_worktree_with_agent(env, workmux_exe_path, mux_repo_path, "feature-run-io")

    result = run_workmux_run(
        env,
        workmux_exe_path,
        mux_repo_path,
        "run feature-run-io -- sh -c 'echo OUT_MARKER; echo ERR_MARKER >&2'",
    )
    assert result.exit_code == 0
    assert "OUT_MARKER" in result.stdout
    assert "ERR_MARKER" in result.stderr


def test_run_nonzero_exit_code_propagates(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Run exits with the child command's exit code."""
    env = mux_server
    setup_worktree_with_agent(env, workmux_exe_path, mux_repo_path, "feature-run-exit")

    result = run_workmux_run(
        env,
        workmux_exe_path,
        mux_repo_path,
        "run feature-run-exit -- sh -c 'exit 42'",
        expect_fail=True,
    )
    assert result.exit_code == 42


def test_run_keep_preserves_artifacts(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Run with --keep preserves spec.json and result.json in the run directory."""
    env = mux_server
    setup_worktree_with_agent(env, workmux_exe_path, mux_repo_path, "feature-run-keep")

    result = run_workmux_run(
        env,
        workmux_exe_path,
        mux_repo_path,
        "run feature-run-keep --keep -- echo KEPT_OUTPUT",
    )
    assert result.exit_code == 0

    run_dir = extract_artifacts_path(result.stderr)
    assert run_dir.exists(), f"Run directory should be preserved: {run_dir}"

    # Verify spec.json
    spec = json.loads((run_dir / "spec.json").read_text())
    assert "echo" in spec["command"]
    assert "worktree_path" in spec

    # Verify result.json
    run_result = json.loads((run_dir / "result.json").read_text())
    assert run_result["exit_code"] == 0

    # Verify stdout was captured to file
    stdout_content = (run_dir / "stdout").read_text()
    assert "KEPT_OUTPUT" in stdout_content


def test_run_timeout_exits_124(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Run with --timeout exits with code 124 when the command exceeds the limit."""
    env = mux_server
    setup_worktree_with_agent(env, workmux_exe_path, mux_repo_path, "feature-run-to")

    result = run_workmux_run(
        env,
        workmux_exe_path,
        mux_repo_path,
        "run feature-run-to --timeout 1 -- sleep 30",
        expect_fail=True,
    )
    assert result.exit_code == 124
    assert "Timeout" in result.stderr
