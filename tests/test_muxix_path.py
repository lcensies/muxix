from pathlib import Path

from .conftest import (
    MuxEnvironment,
    get_worktree_path,
    run_muxix_add,
    run_muxix_command,
    write_muxix_config,
)


def test_path_returns_worktree_path(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix path` returns the correct path for an existing worktree."""
    env = mux_server
    branch_name = "feature-test"
    write_muxix_config(mux_repo_path)
    run_muxix_add(env, muxix_exe_path, mux_repo_path, branch_name)

    result = run_muxix_command(
        env, muxix_exe_path, mux_repo_path, f"path {branch_name}"
    )

    expected_path = get_worktree_path(mux_repo_path, branch_name)
    assert result.stdout.strip() == str(expected_path)


def test_path_fails_for_nonexistent_worktree(
    mux_server: MuxEnvironment, muxix_exe_path: Path, mux_repo_path: Path
):
    """Verifies `muxix path` fails with non-zero exit code for nonexistent worktree."""
    env = mux_server

    result = run_muxix_command(
        env,
        muxix_exe_path,
        mux_repo_path,
        "path nonexistent-branch",
        expect_fail=True,
    )

    assert result.exit_code != 0
    assert "not found" in result.stderr.lower() or "worktree" in result.stderr.lower()
