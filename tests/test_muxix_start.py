"""E2E for project tracking (`muxix project ...`) and `muxix start`.

* `project add/list/rm` — registry roundtrip in the isolated HOME.
* `muxix add <repo-dir>` sugar — a directory containing .git is tracked as a
  project instead of creating a worktree.
* `muxix start` — creates one session per tracked project plus a window per
  worktree, and is idempotent on a second run.
"""

from pathlib import Path

import pytest

from .conftest import (
    MuxEnvironment,
    get_window_name,
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
