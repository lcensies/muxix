"""
Tests for agent state management.

Verifies that:
1. workmux set-window-status creates agent state files
2. State files contain correct fields (pane_key, pane_pid, command, workdir, status)
3. State files contain pane identification info needed for reconciliation
4. Status updates overwrite existing state files (no duplicates)

Note: Reconciliation behavior (removing stale state files) is tested implicitly
through the dashboard TUI, which calls load_reconciled_agents(). These tests
verify the state files have the fields needed for reconciliation to work.
"""

import json
import time
from pathlib import Path


from .conftest import (
    MuxEnvironment,
    get_window_name,
    make_env_script,
    poll_until,
    run_workmux_add,
    write_workmux_config,
)


def get_state_dir(env: MuxEnvironment) -> Path:
    """Get the workmux state directory for this test environment."""
    return Path(env.env["XDG_STATE_HOME"]) / "workmux"


def get_agents_dir(env: MuxEnvironment) -> Path:
    """Get the agents state directory."""
    return get_state_dir(env) / "agents"


def list_agent_state_files(env: MuxEnvironment) -> list[Path]:
    """List all agent state files."""
    agents_dir = get_agents_dir(env)
    if not agents_dir.exists():
        return []
    return list(agents_dir.glob("*.json"))


def read_agent_state(path: Path) -> dict:
    """Read and parse an agent state file."""
    return json.loads(path.read_text())


def build_status_cmd(env: MuxEnvironment, workmux_exe: Path, status: str) -> str:
    """Build a set-window-status command with proper env vars for test isolation.

    The command runs inside a pane's shell, which doesn't inherit the test's
    XDG_STATE_HOME. We need to explicitly pass it so state files go to the
    test's isolated directory.

    Returns a path to a script file (to avoid tmux send-keys line length limits).
    """
    command = f"{workmux_exe} set-window-status {status}"
    return make_env_script(env, command, {"XDG_STATE_HOME": env.env["XDG_STATE_HOME"]})


# -----------------------------------------------------------------------------
# Tests
# -----------------------------------------------------------------------------


def test_set_window_status_creates_state_file(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies that workmux set-window-status creates an agent state file."""
    env = mux_server
    branch_name = "feature-state-test"
    window_name = get_window_name(branch_name)

    # Configure with a pane that starts a shell (no blocking command)
    # so we can send keys to it
    write_workmux_config(
        mux_repo_path,
        panes=[
            {"focus": True},  # Just a shell, no command
        ],
    )

    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)

    # Wait for window/shell to be ready
    time.sleep(1.5)

    # Send set-window-status command to the pane using tab title
    # This simulates what Claude hooks do when agent starts working
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)

    # Wait for state file to be created
    def state_file_exists():
        files = list_agent_state_files(env)
        return len(files) > 0

    assert poll_until(state_file_exists, timeout=5.0), (
        f"No agent state file created after set-window-status. "
        f"State dir: {get_agents_dir(env)}"
    )


def test_state_file_has_correct_fields(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies that agent state files contain the expected fields."""
    env = mux_server
    branch_name = "feature-fields-test"
    window_name = get_window_name(branch_name)

    write_workmux_config(
        mux_repo_path,
        panes=[
            {"focus": True},  # Just a shell
        ],
    )

    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Trigger state file creation using tab title
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)

    def state_file_exists():
        return len(list_agent_state_files(env)) > 0

    assert poll_until(state_file_exists, timeout=5.0), "State file not created"

    # Read and verify state file contents
    state_files = list_agent_state_files(env)
    state = read_agent_state(state_files[0])

    # Check required fields exist
    assert "pane_key" in state, "Missing pane_key field"
    assert "pane_pid" in state, "Missing pane_pid field"
    assert "command" in state, "Missing command field"
    assert "workdir" in state, "Missing workdir field"
    assert "status" in state, "Missing status field"

    # Check pane_key structure
    pane_key = state["pane_key"]
    assert "backend" in pane_key, "Missing backend in pane_key"
    assert "instance" in pane_key, "Missing instance in pane_key"
    assert "pane_id" in pane_key, "Missing pane_id in pane_key"

    # Verify values are sensible
    assert pane_key["backend"] == env.backend_name
    assert state["pane_pid"] > 0, "pane_pid should be positive"
    assert state["status"] == "working", (
        f"Expected status 'working', got '{state['status']}'"
    )
    # Command could be "workmux" (if captured during set-window-status) or the shell
    assert state["command"], "command should not be empty"


def test_state_file_contains_pane_info(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies that state file contains valid pane identification info.

    This tests that the state file has the information needed for reconciliation
    to detect stale panes (pane_id, pane_pid, command).
    """
    env = mux_server
    branch_name = "feature-pane-info-test"
    window_name = get_window_name(branch_name)

    write_workmux_config(
        mux_repo_path,
        panes=[
            {"focus": True},  # Just a shell
        ],
    )

    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create state file
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)

    def state_file_exists():
        return len(list_agent_state_files(env)) > 0

    assert poll_until(state_file_exists, timeout=5.0), "State file not created"

    # Read state file and verify reconciliation-relevant fields
    state_files = list_agent_state_files(env)
    state = read_agent_state(state_files[0])

    # Verify pane_key has all required fields for pane identification
    pane_key = state["pane_key"]
    assert pane_key["pane_id"], "pane_id should be set"

    # Verify we have PID and command for stale detection
    assert state["pane_pid"] > 0, (
        "pane_pid should be positive (for PID-based stale detection)"
    )
    assert state["command"], "command should be set (for command-change detection)"

    # Verify workdir is set (useful for context)
    assert state["workdir"], "workdir should be set"


def test_status_update_overwrites_state(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Verifies that calling set-window-status again updates the existing state.

    This ensures the state file is updated (not duplicated) when status changes.
    """
    env = mux_server
    branch_name = "feature-status-update-test"
    window_name = get_window_name(branch_name)

    write_workmux_config(
        mux_repo_path,
        panes=[
            {"focus": True},  # Just a shell
        ],
    )

    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch_name)
    time.sleep(1.5)

    # Create initial state with "working" status
    status_cmd = build_status_cmd(env, workmux_exe_path, "working")
    env.send_keys(window_name, status_cmd)

    def state_file_exists():
        return len(list_agent_state_files(env)) > 0

    assert poll_until(state_file_exists, timeout=5.0), "State file not created"

    # Verify initial status
    state_files = list_agent_state_files(env)
    assert len(state_files) == 1, f"Expected 1 state file, got {len(state_files)}"
    state = read_agent_state(state_files[0])
    assert state["status"] == "working", f"Expected 'working', got '{state['status']}'"

    # Wait a bit and update to "done" status
    time.sleep(0.5)
    status_cmd = build_status_cmd(env, workmux_exe_path, "done")
    env.send_keys(window_name, status_cmd)

    # Poll for status to be updated (more reliable than fixed sleep under load)
    def status_is_done():
        files = list_agent_state_files(env)
        if not files:
            return False
        try:
            state = read_agent_state(files[0])
            return state.get("status") == "done"
        except json.JSONDecodeError:
            # File might be partially written, keep polling
            return False

    assert poll_until(status_is_done, timeout=5.0), "Status was not updated to 'done'"

    # Should still be exactly 1 state file (updated, not duplicated)
    state_files = list_agent_state_files(env)
    assert len(state_files) == 1, (
        f"Expected 1 state file after update, got {len(state_files)}"
    )
