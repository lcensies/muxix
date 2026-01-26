"""Fixtures specific to `workmux add` command tests."""

from pathlib import Path

import pytest

from ..conftest import (
    MuxEnvironment,
    get_worktree_path,
    run_workmux_command,
    write_workmux_config,
)


def add_branch_and_get_worktree(
    env: MuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    branch_name: str,
    extra_args: str = "",
    command_target: str | None = None,
    **kwargs,
) -> Path:
    """Run `workmux add` and return the new worktree path."""
    target = command_target or branch_name
    command = f"add {target}"
    if extra_args:
        command = f"{command} {extra_args}"

    run_workmux_command(
        env,
        workmux_exe_path,
        repo_path,
        command,
        **kwargs,
    )

    worktree_path = get_worktree_path(repo_path, branch_name)
    assert worktree_path.is_dir()
    return worktree_path


@pytest.fixture
def mux_add_worktree(mux_server, workmux_exe_path, mux_repo_path):
    """Factory fixture to add worktrees with less boilerplate (multibackend)."""

    def _add(
        branch_name: str,
        extra_args: str = "",
        command_target: str | None = None,
        **kwargs,
    ) -> Path:
        return add_branch_and_get_worktree(
            mux_server,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=extra_args,
            command_target=command_target,
            **kwargs,
        )

    return _add


@pytest.fixture
def mux_setup_workmux_config(mux_repo_path):
    """Factory fixture to write workmux config with less boilerplate (multibackend)."""

    def _setup(**kwargs):
        write_workmux_config(mux_repo_path, **kwargs)

    return _setup
