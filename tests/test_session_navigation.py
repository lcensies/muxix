"""Tests for session navigation scenarios during cleanup operations.

These tests verify that session-mode cleanup generates correct navigation
commands in the deferred scripts. The headless test environment cannot fully
test client-attached navigation (switch-client requires a PTY-attached
terminal), but we can verify:

  1. The deferred script content is correct (via RUST_LOG capture)
  2. The end state is correct (session killed, worktree cleaned up)

For full navigation testing (verifying the user actually lands on the right
session after remove/merge), manual testing is required because tmux
switch-client needs a client that headless test environments don't provide.

Pseudo-client attachment via `script -q /dev/null tmux attach` is possible
but fragile and platform-dependent, so we test script content instead.

Mixed-mode navigation (session worktree navigating to a window-mode target
during merge) also requires manual testing: tmux's window_exists_by_full_name
only checks the current session's windows, so a window-mode target in another
session can't be detected when running from inside the source session.
"""

import shlex
import time
from pathlib import Path

import pytest

from .conftest import (
    TmuxEnvironment,
    WorkmuxCommandResult,
    assert_session_exists,
    assert_session_not_exists,
    get_scripts_dir,
    get_session_name,
    get_worktree_path,
    poll_until_file_has_content,
    run_workmux_command,
    write_workmux_config,
)
from .test_workmux_add.conftest import add_branch_and_get_worktree

# Session navigation is tmux-specific (other backends don't support sessions)
pytestmark = pytest.mark.tmux_only

# Time to wait for deferred scripts to complete (300ms sleep + execution)
DEFERRED_SCRIPT_WAIT = 2.0


def run_workmux_in_session(
    env: TmuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    session_name: str,
    command: str,
    working_dir: Path | None = None,
    rust_log: str | None = None,
) -> WorkmuxCommandResult:
    """Run a workmux command from inside a specific tmux session.

    Unlike run_workmux_command which runs from the test session, this sends
    the command to a pane inside the target session. This allows testing
    behavior that depends on being inside the session (e.g.,
    is_inside_matching_target detection in cleanup.rs).

    The command is executed via a script file to avoid tmux send-keys line
    length limits. Exit code, stdout, and stderr are captured to files and
    returned after the command completes.

    When rust_log is set, XDG_STATE_HOME is redirected to a temp directory
    so the log file can be read back. workmux writes tracing output to a
    log file (not stderr), so the log content is appended to stderr in the
    returned result.

    Note: After the command completes, the session may be killed by deferred
    scripts (e.g., after workmux remove). The exit code and output are
    captured before deferred scripts run (they have a 300ms sleep), so they
    are still available.
    """
    scripts_dir = get_scripts_dir(env)
    stdout_file = scripts_dir / "in_session_stdout.txt"
    stderr_file = scripts_dir / "in_session_stderr.txt"
    exit_code_file = scripts_dir / "in_session_exit.txt"
    script_file = scripts_dir / "in_session_run.sh"
    state_dir = scripts_dir / "xdg_state"
    state_dir.mkdir(exist_ok=True)

    for f in [stdout_file, stderr_file, exit_code_file]:
        if f.exists():
            f.unlink()

    # Clear any previous log file so we only see output from this run
    log_file = state_dir / "workmux" / "workmux.log"
    if log_file.exists():
        log_file.unlink()

    workdir = working_dir or repo_path

    rust_log_line = ""
    if rust_log:
        rust_log_line = f"export RUST_LOG={shlex.quote(rust_log)}"

    script_content = f"""#!/bin/sh
trap 'echo $? > {shlex.quote(str(exit_code_file))}' EXIT
export PATH={shlex.quote(env.env["PATH"])}
export TMPDIR={shlex.quote(env.env.get("TMPDIR", "/tmp"))}
export HOME={shlex.quote(env.env.get("HOME", ""))}
export XDG_STATE_HOME={shlex.quote(str(state_dir))}
export WORKMUX_TEST=1
{rust_log_line}
cd {shlex.quote(str(workdir))}
{shlex.quote(str(workmux_exe_path))} {command} > {shlex.quote(str(stdout_file))} 2> {shlex.quote(str(stderr_file))}
"""
    script_file.write_text(script_content)
    script_file.chmod(0o755)

    # Send the script to the target session's pane (not the test session)
    env.send_keys(f"={session_name}:", str(script_file))

    # Wait for completion (longer timeout to account for deferred scripts)
    if not poll_until_file_has_content(exit_code_file, timeout=10.0):
        pane_content = (
            env.tmux(
                ["capture-pane", "-p", "-t", f"={session_name}:"], check=False
            ).stdout
            or "(session may have been killed)"
        )
        raise AssertionError(
            f"Command did not complete in time (session: {session_name})\n"
            f"Pane content:\n{pane_content}"
        )

    stderr_content = stderr_file.read_text() if stderr_file.exists() else ""

    # Append workmux log file content to stderr when RUST_LOG was set.
    # workmux writes tracing output to $XDG_STATE_HOME/workmux/workmux.log
    # (not stderr), so we read it back and include it in the result.
    if rust_log and log_file.exists():
        stderr_content += log_file.read_text()

    return WorkmuxCommandResult(
        exit_code=int(exit_code_file.read_text().strip()),
        stdout=stdout_file.read_text() if stdout_file.exists() else "",
        stderr=stderr_content,
    )


class TestRemoveFromInsideSession:
    """Tests for removing session-mode worktrees from inside the session.

    When running `workmux remove` from inside a session-mode worktree,
    the deferred script should switch to the last session before killing
    the source session, so the user returns to where they were previously
    instead of tmux picking an arbitrary session.

    Limitation: These tests verify the deferred script content and end state
    but cannot verify the actual client navigation because switch-client
    requires an attached terminal (PTY). Full navigation testing requires
    manual verification with an attached tmux client.
    """

    def test_remove_generates_switch_to_last_session(
        self, mux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
    ):
        """Verify remove from inside session generates switch-client -l.

        The deferred cleanup script should contain `switch-client -l` before
        `kill-session` so that the user's client returns to their previous
        session rather than tmux choosing an arbitrary one.
        """
        env = mux_server
        branch_name = "feature-session-nav-switch"
        session_name = get_session_name(branch_name)

        write_workmux_config(repo_path)

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            repo_path,
            branch_name,
            extra_args="--session --background",
        )
        assert_session_exists(env, session_name)

        # Run remove from INSIDE the session with debug logging to capture
        # the deferred script content
        result = run_workmux_in_session(
            env,
            workmux_exe_path,
            repo_path,
            session_name,
            "remove -f",
            working_dir=worktree_path,
            rust_log="workmux=debug",
        )

        assert result.exit_code == 0, f"Remove failed: {result.stderr}"

        # Verify the deferred script contains switch-client -l.
        # The debug log includes the full script string in a line containing
        # "kill_only_script" (when target doesn't exist) or
        # "nav_and_kill_script" (when target exists and we navigate to it).
        assert "switch-client -l" in result.stderr, (
            f"Expected 'switch-client -l' in deferred script.\n"
            f"Debug output:\n{result.stderr}"
        )

        # switch-client -l must come BEFORE kill-session in the script
        # to ensure the client switches away before the session is destroyed
        switch_pos = result.stderr.find("switch-client -l")
        kill_pos = result.stderr.find("kill-session")
        assert kill_pos > 0, (
            f"Expected 'kill-session' in deferred script.\n"
            f"Debug output:\n{result.stderr}"
        )
        assert switch_pos < kill_pos, (
            f"switch-client -l (pos {switch_pos}) should come before "
            f"kill-session (pos {kill_pos}) in the deferred script"
        )

        # Wait for deferred script to complete
        time.sleep(DEFERRED_SCRIPT_WAIT)

        # Verify end state: session killed and worktree removed
        assert_session_not_exists(env, session_name)
        assert not worktree_path.exists(), (
            "Worktree should be removed by deferred cleanup"
        )


class TestCreateAndSwitch:
    """Tests for `workmux add --session` without --background.

    When creating a session-mode worktree without --background, workmux
    should attempt to switch the client to the new session. In a headless
    test environment, switch-client will fail (no PTY-attached client),
    but we can verify the session is created regardless.

    Full switch verification requires manual testing with an attached
    terminal. The key behavior to verify headlessly is that the session
    is created even when the switch attempt fails.
    """

    def test_add_session_creates_session_despite_switch_failure(
        self, mux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
    ):
        """Verify --session without --background creates the session.

        In a headless environment, the switch-client call fails because
        there's no attached client. The session should still be created
        and the worktree should still be set up correctly.
        """
        env = mux_server
        branch_name = "feature-session-no-bg"
        session_name = get_session_name(branch_name)
        worktree_path = get_worktree_path(repo_path, branch_name)

        write_workmux_config(repo_path)

        # Run without --background; may fail due to switch-client
        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            f"add {branch_name} --session",
            expect_fail=True,
        )

        # The session should be created even if the switch failed
        sessions_result = env.tmux(
            ["list-sessions", "-F", "#{session_name}"], check=False
        )
        existing_sessions = [s for s in sessions_result.stdout.strip().split("\n") if s]

        assert session_name in existing_sessions, (
            f"Session {session_name!r} should exist after 'add --session' "
            f"even when switch-client fails.\n"
            f"Existing sessions: {existing_sessions!r}\n"
            f"Command stderr: {result.stderr}"
        )

        # Worktree should also be created
        assert worktree_path.is_dir(), (
            f"Worktree at {worktree_path} should exist after 'add --session'"
        )
