"""Sidebar modal actions, end to end: the create form and the delete overlay.

Drives a real sidebar pane: `muxix sidebar` is enabled inside the isolated tmux
server, keys are sent to the sidebar's own pane, and the pane is captured to see
what it drew.
"""

from pathlib import Path

import pytest

from .conftest import (
    MuxEnvironment,
    TmuxEnvironment,
    get_window_name,
    get_worktree_path,
    make_env_script,
    poll_until,
    run_muxix_command,
    wait_for_window_ready,
    write_muxix_config,
)

# The sidebar is tmux-only: its panes are marked with a tmux pane option.
pytestmark = pytest.mark.tmux_only


def sidebar_pane_in(
    env: TmuxEnvironment, window_name: str, timeout: float = 15.0
) -> str:
    """The pane id of the sidebar pane in `window_name`."""
    found: list[str] = []

    def ready() -> bool:
        result = env.tmux(
            ["list-panes", "-t", window_name, "-F", "#{pane_id} #{@muxix_role}"],
            check=False,
        )
        for line in result.stdout.splitlines():
            parts = line.split()
            if len(parts) == 2 and parts[1] == "sidebar":
                found.append(parts[0])
                return True
        return False

    assert poll_until(ready, timeout=timeout), (
        f"no sidebar pane appeared in {window_name!r}"
    )
    return found[0]


def pane_text(env: TmuxEnvironment, pane_id: str) -> str:
    result = env.tmux(["capture-pane", "-p", "-t", pane_id], check=False)
    return result.stdout if result.returncode == 0 else ""


def wait_for_pane_text(
    env: TmuxEnvironment, pane_id: str, text: str, timeout: float = 15.0
) -> None:
    last = ""

    def ready() -> bool:
        nonlocal last
        last = pane_text(env, pane_id)
        return text in last

    assert poll_until(ready, timeout=timeout), (
        f"{text!r} never appeared in pane {pane_id}.\n--- PANE ---\n{last}\n------------"
    )


@pytest.fixture
def sidebar_session(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """One agent, with the sidebar enabled in every window.

    Yields the sidebar pane of the agent's own window, whose selection follows
    that window's agent.
    """
    env = mux_server
    assert isinstance(env, TmuxEnvironment)
    write_muxix_config(mux_repo_path)

    handle = "sidebar-host"
    window_name = get_window_name(handle)
    run_muxix_command(env, muxix_exe_path, mux_repo_path, f"add {handle}")
    wait_for_window_ready(env, window_name)

    # Only an agent that reported a status is in the state store, which is
    # where the daemon reads the sidebar's rows from.
    env.send_keys(
        window_name,
        make_env_script(
            env,
            f"{muxix_exe_path} set-window-status working",
            {"XDG_STATE_HOME": env.env["XDG_STATE_HOME"]},
        ),
    )

    run_muxix_command(env, muxix_exe_path, mux_repo_path, "sidebar")
    pane_id = sidebar_pane_in(env, window_name)
    # The daemon's first snapshot is what puts the agent in the list.
    wait_for_pane_text(env, pane_id, handle)

    try:
        yield env, pane_id, handle, window_name
    finally:
        # The daemon outlives the tmux server, so stop it explicitly.
        run_muxix_command(
            env, muxix_exe_path, mux_repo_path, "daemon stop", expect_fail=False
        )


def test_the_delete_overlay_is_drawn_in_the_sidebar_pane(sidebar_session):
    """`d` draws the centered dialog inside the left sidebar, not a tmux prompt."""
    env, pane_id, _handle, window_name = sidebar_session

    env.tmux(["send-keys", "-t", pane_id, "d"])
    wait_for_pane_text(env, pane_id, "Delete agent")

    drawn = pane_text(env, pane_id)
    assert "─" in drawn or "│" in drawn, f"the dialog has no border:\n{drawn}"

    # Cancelling closes the dialog and removes nothing.
    env.tmux(["send-keys", "-t", pane_id, "Escape"])
    assert poll_until(lambda: "Delete agent" not in pane_text(env, pane_id)), (
        "Esc must close the dialog"
    )
    assert window_name in env.list_windows(), "cancel must delete nothing"


def test_creating_an_agent_through_the_form(
    sidebar_session, muxix_exe_path: Path, mux_repo_path: Path
):
    """`a`, a branch name and Enter create a window and a worktree."""
    env, pane_id, _handle, _window_name = sidebar_session
    branch = "from-sidebar"

    env.tmux(["send-keys", "-t", pane_id, "a"])
    wait_for_pane_text(env, pane_id, "New agent")
    # The repository field is prefilled from the selected row, so the branch
    # name is all the form needs.
    env.tmux(["send-keys", "-t", pane_id, "-l", branch])
    env.tmux(["send-keys", "-t", pane_id, "Enter"])

    created = get_window_name(branch)
    assert poll_until(lambda: created in env.list_windows(), timeout=30.0), (
        f"no window {created!r} appeared.\n--- PANE ---\n{pane_text(env, pane_id)}"
    )
    worktree = get_worktree_path(mux_repo_path, branch)
    assert poll_until(worktree.is_dir, timeout=30.0), f"no worktree at {worktree}"
