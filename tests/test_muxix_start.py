"""E2E for project tracking (`muxix project ...`) and `muxix start`.

* `project add/list/rm` — registry roundtrip in the isolated HOME.
* `muxix add <repo-dir>` sugar — a directory containing .git is tracked as a
  project instead of creating a worktree.
* `muxix start` — creates one session per tracked project plus a window per
  worktree, and is idempotent on a second run.
* `project_open` / `--worktrees` — which worktrees `start` restores.
"""

import json
import time
from pathlib import Path

import pytest

from .conftest import (
    MuxEnvironment,
    get_window_name,
    get_worktree_path,
    run_muxix_add,
    run_muxix_command,
    write_muxix_config,
)


def _session_names(env: MuxEnvironment) -> list[str]:
    result = env.mux_command(["list-sessions", "-F", "#{session_name}"])
    return [s for s in result.stdout.strip().split("\n") if s]


def _windows_of(env: MuxEnvironment, session: str) -> list[str]:
    result = env.mux_command(
        ["list-windows", "-t", session, "-F", "#{window_name}"], check=False
    )
    if result.returncode != 0:
        return []
    return [w for w in result.stdout.strip().split("\n") if w]


def test_project_registry_roundtrip(
    mux_server: MuxEnvironment, muxix_exe_path: Path, repo_path: Path
):
    env = mux_server
    name = repo_path.name

    result = run_muxix_command(
        env, muxix_exe_path, repo_path, f"project add {repo_path}"
    )
    assert "Tracking project" in result.stdout

    # Idempotent
    result = run_muxix_command(
        env, muxix_exe_path, repo_path, f"project add {repo_path}"
    )
    assert "already tracked" in result.stdout

    result = run_muxix_command(env, muxix_exe_path, repo_path, "project list")
    assert name in result.stdout

    result = run_muxix_command(env, muxix_exe_path, repo_path, f"project rm {name}")
    assert "Untracked" in result.stdout

    result = run_muxix_command(env, muxix_exe_path, repo_path, "project list")
    assert name not in result.stdout


def test_add_directory_sugar_tracks_project(
    mux_server: MuxEnvironment, muxix_exe_path: Path, repo_path: Path
):
    """`muxix add <dir-with-.git>` registers a project, no worktree."""
    env = mux_server

    result = run_muxix_command(env, muxix_exe_path, repo_path, f"add {repo_path}")
    assert "Tracking project" in result.stdout

    result = run_muxix_command(env, muxix_exe_path, repo_path, "project list")
    assert repo_path.name in result.stdout

    # No worktree was created
    result = run_muxix_command(env, muxix_exe_path, repo_path, "list")
    assert str(repo_path.name) not in [
        line.split()[0]
        for line in result.stdout.strip().split("\n")[1:]
        if line.strip()
    ]


@pytest.mark.tmux_only
def test_start_creates_session_and_worktree_windows_idempotently(
    mux_server: MuxEnvironment, muxix_exe_path: Path, repo_path: Path
):
    env = mux_server
    project = repo_path.name
    branch = "feat-start"
    window = get_window_name(branch)

    write_muxix_config(repo_path)
    run_muxix_command(env, muxix_exe_path, repo_path, f"project add {repo_path}")

    # Create a worktree, then close its window so start has work to do
    run_muxix_add(env, muxix_exe_path, repo_path, branch)
    run_muxix_command(env, muxix_exe_path, repo_path, f"close {branch}")

    result = run_muxix_command(env, muxix_exe_path, repo_path, "start")
    assert "started session" in result.stdout

    assert project in _session_names(env)
    assert window in _windows_of(env, project)

    # Second run: no-op, session intact, no duplicate windows
    result = run_muxix_command(env, muxix_exe_path, repo_path, "start")
    assert "already running" in result.stdout
    assert _windows_of(env, project).count(window) == 1


@pytest.mark.tmux_only
def test_start_skips_missing_project_dirs(
    mux_server: MuxEnvironment, muxix_exe_path: Path, repo_path: Path
):
    env = mux_server
    ghost = repo_path.parent / "ghost-project"
    ghost.mkdir()
    run_muxix_command(env, muxix_exe_path, repo_path, f"project add {ghost}")
    ghost.rmdir()

    result = run_muxix_command(env, muxix_exe_path, repo_path, "start")
    assert "no longer exists" in result.stdout + result.stderr


def _seed_agent_state(env: MuxEnvironment, workdir: Path, pane_id: str) -> None:
    """Write one agent state file claiming an agent ran in `workdir`.

    `instance` must match `Multiplexer::instance_id`, which for tmux is the
    socket path from `$TMUX`. The file name is irrelevant — the store scans
    every `*.json` and reads the `pane_key` inside.
    """
    now = int(time.time())
    state = {
        "agent_id": f"seeded-{pane_id.strip('%')}",
        "pane_key": {
            "backend": "tmux",
            "instance": str(env.socket_path),
            "pane_id": pane_id,
        },
        "workdir": str(workdir),
        "status": "waiting",
        "status_ts": now,
        "pane_title": None,
        "pane_pid": 1,
        "command": "node",
        "updated_ts": now,
    }
    agents_dir = env.home_path / ".local" / "state" / "muxix" / "agents"
    agents_dir.mkdir(parents=True, exist_ok=True)
    (agents_dir / f"seeded-{pane_id.strip('%')}.json").write_text(json.dumps(state))


@pytest.mark.tmux_only
def test_start_opens_only_worktrees_an_agent_ran_in(
    mux_server: MuxEnvironment, muxix_exe_path: Path, repo_path: Path
):
    """Default `project_open: active`, plus the `--worktrees all` escape hatch."""
    env = mux_server
    project = repo_path.name
    seeded, bare = "feat-seeded", "feat-bare"

    write_muxix_config(repo_path)
    run_muxix_command(env, muxix_exe_path, repo_path, f"project add {repo_path}")
    for branch in (seeded, bare):
        run_muxix_add(env, muxix_exe_path, repo_path, branch)
        run_muxix_command(env, muxix_exe_path, repo_path, f"close {branch}")

    _seed_agent_state(env, get_worktree_path(repo_path, seeded), "%901")

    result = run_muxix_command(env, muxix_exe_path, repo_path, "start")
    windows = _windows_of(env, project)
    assert get_window_name(seeded) in windows
    assert get_window_name(bare) not in windows
    assert "skipped 1 worktree(s) (project_open: active)" in result.stdout

    run_muxix_command(env, muxix_exe_path, repo_path, "start --worktrees all")
    assert get_window_name(bare) in _windows_of(env, project)
