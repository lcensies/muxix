import subprocess
from pathlib import Path

from .conftest import (
    DEFAULT_WINDOW_PREFIX,
    MuxEnvironment,
    get_window_name,
    get_worktree_path,
    run_workmux_add,
    run_workmux_command,
    write_workmux_config,
)


def _git(repo: Path, *args: str) -> str:
    out = subprocess.run(
        ["git", *args], cwd=str(repo), capture_output=True, text=True, check=True
    )
    return out.stdout.strip()


def test_rename_renames_worktree_dir_and_tmux_window(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Basic rename: worktree directory and tmux window are renamed, branch untouched."""
    env = mux_server
    branch = "feature-old"
    old_handle = branch
    new_handle = "feature-new"

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch)

    # Sanity: old dir + old window exist.
    old_path = get_worktree_path(mux_repo_path, old_handle)
    new_path = old_path.parent / new_handle
    assert old_path.exists()
    assert get_window_name(old_handle) in env.list_windows()
    assert new_handle not in [p.name for p in old_path.parent.iterdir()]

    run_workmux_command(
        env, workmux_exe_path, mux_repo_path, f"rename {old_handle} {new_handle}"
    )

    # Worktree dir renamed.
    assert not old_path.exists()
    assert new_path.exists()

    # tmux window renamed.
    windows = env.list_windows()
    assert get_window_name(old_handle) not in windows
    assert f"{DEFAULT_WINDOW_PREFIX}{new_handle}" in windows

    # Branch unchanged (we didn't pass --branch).
    branches = _git(mux_repo_path, "branch", "--list")
    assert branch in branches
    assert new_handle not in branches.replace(branch, "")


def test_rename_with_branch_also_renames_branch(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """`--branch` also renames the underlying git branch."""
    env = mux_server
    branch = "feature-rename-branch"
    new_handle = "feature-shiny"

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch)

    run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        f"rename {branch} {new_handle} --branch",
    )

    branches = _git(mux_repo_path, "branch", "--list")
    assert branch not in branches.split()
    assert new_handle in branches

    # The worktree path checkout is the new branch.
    checkout = _git(
        mux_repo_path.parent / f"{mux_repo_path.name}__worktrees" / new_handle,
        "rev-parse",
        "--abbrev-ref",
        "HEAD",
    )
    assert checkout == new_handle


def test_rename_rejects_existing_target_path(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Rename fails when the new path already belongs to another worktree."""
    env = mux_server

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, "feat-a")
    run_workmux_add(env, workmux_exe_path, mux_repo_path, "feat-b")

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        "rename feat-a feat-b",
        expect_fail=True,
    )
    assert "already exists" in result.stderr or "already exists" in result.stdout


def test_rename_rejects_main_worktree(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Rename fails when trying to rename the main worktree."""
    env = mux_server

    write_workmux_config(mux_repo_path)
    # mux_repo_path is the main worktree; its handle is the dir basename.
    main_handle = mux_repo_path.name

    result = run_workmux_command(
        env,
        workmux_exe_path,
        mux_repo_path,
        f"rename {main_handle} whatever",
        expect_fail=True,
    )
    # Could fail with either "main worktree" or "not found" depending on
    # how the main worktree is registered vs lookup.
    stderr = result.stderr + result.stdout
    assert "main worktree" in stderr or "not found" in stderr


def test_rename_preserves_uncommitted_changes(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Rename is non-destructive: uncommitted changes survive the rename."""
    env = mux_server
    branch = "feature-dirty"
    new_handle = "feature-dirty-renamed"

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch)

    old_path = get_worktree_path(mux_repo_path, branch)
    dirty_file = old_path / "dirty.txt"
    dirty_file.write_text("work in progress")

    run_workmux_command(
        env, workmux_exe_path, mux_repo_path, f"rename {branch} {new_handle}"
    )

    new_path = old_path.parent / new_handle
    moved_dirty = new_path / "dirty.txt"
    assert moved_dirty.exists()
    assert moved_dirty.read_text() == "work in progress"


def test_rename_accepts_branch_name_as_target(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """Rename accepts either the handle or the branch as the source target.

    When the user passes a branch that differs from the handle (e.g. because
    of `worktree_prefix` or slugification), the workflow must still derive
    the true handle from the worktree directory basename so per-handle
    state migrates correctly.
    """
    env = mux_server
    branch = "feature/with-slash"
    # With default naming (Full), handle slugifies to "feature-with-slash".
    old_handle = "feature-with-slash"
    new_handle = "feature-renamed"

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch)

    # Verify the handle-keyed metadata exists before rename.
    old_mode = _git(
        mux_repo_path,
        "config",
        "--local",
        "--get",
        f"workmux.worktree.{old_handle}.mode",
    )
    assert old_mode in ("window", "session")

    # Pass the BRANCH name (not the handle) as the rename target.
    run_workmux_command(
        env, workmux_exe_path, mux_repo_path, f"rename {branch} {new_handle}"
    )

    # Metadata should have migrated to the new handle-keyed section.
    new_mode = _git(
        mux_repo_path,
        "config",
        "--local",
        "--get",
        f"workmux.worktree.{new_handle}.mode",
    )
    assert new_mode == old_mode

    # Old metadata section is gone.
    probe = subprocess.run(
        [
            "git",
            "config",
            "--local",
            "--get",
            f"workmux.worktree.{old_handle}.mode",
        ],
        cwd=str(mux_repo_path),
        capture_output=True,
        text=True,
    )
    assert probe.returncode != 0


def test_rename_migrates_worktree_metadata(
    mux_server: MuxEnvironment, workmux_exe_path: Path, mux_repo_path: Path
):
    """`workmux.worktree.<old>.mode` migrates to `workmux.worktree.<new>.mode`."""
    env = mux_server
    branch = "feature-meta"
    new_handle = "feature-meta-new"

    write_workmux_config(mux_repo_path)
    run_workmux_add(env, workmux_exe_path, mux_repo_path, branch)

    # Old metadata should exist.
    old_mode = _git(
        mux_repo_path,
        "config",
        "--local",
        "--get",
        f"workmux.worktree.{branch}.mode",
    )
    assert old_mode in ("window", "session")

    run_workmux_command(
        env, workmux_exe_path, mux_repo_path, f"rename {branch} {new_handle}"
    )

    # New metadata exists, old metadata is gone.
    new_mode = _git(
        mux_repo_path,
        "config",
        "--local",
        "--get",
        f"workmux.worktree.{new_handle}.mode",
    )
    assert new_mode == old_mode

    # Old key should no longer exist (git config exits 1 if unset).
    probe = subprocess.run(
        [
            "git",
            "config",
            "--local",
            "--get",
            f"workmux.worktree.{branch}.mode",
        ],
        cwd=str(mux_repo_path),
        capture_output=True,
        text=True,
    )
    assert probe.returncode != 0, "Old worktree metadata key should be removed"
