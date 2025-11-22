"""Tests for tmux pane-base-index configuration compatibility."""

from pathlib import Path


from .conftest import (
    TmuxEnvironment,
    get_window_name,
    run_workmux_add,
    write_workmux_config,
)


def test_pane_base_index_1_works_with_pane_ids(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """
    Verifies that workmux works correctly with pane-base-index 1 using pane IDs.

    This test configures tmux with pane-base-index 1 (making panes 1-indexed instead
    of 0-indexed) and verifies that workmux successfully creates panes using pane IDs.
    """
    env = isolated_tmux_server
    branch_name = "test-pane-index"
    window_name = get_window_name(branch_name)

    # Configure tmux with pane-base-index 1 (user's configuration)
    env.tmux(["set-option", "-g", "pane-base-index", "1"])

    # Also test with base-index 1 which the user uses (for windows, not panes)
    env.tmux(["set-option", "-g", "base-index", "1"])

    # Configure workmux with panes and a command to trigger respawn-pane
    write_workmux_config(
        repo_path,
        panes=[
            {"command": "echo 'hello'", "focus": True},
            {"split": "horizontal", "size": 30},
        ],
    )

    # This should now succeed with the pane ID fix
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Verify the window was created
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows

    # Verify all panes were created (should have 2 panes)
    pane_count = env.tmux(
        ["list-panes", "-t", window_name, "-F", "#{pane_id}"]
    ).stdout.splitlines()
    assert len(pane_count) == 2, f"Expected 2 panes, got {len(pane_count)}"


def test_pane_base_index_1_with_multiple_panes(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """
    Verifies that workmux works correctly with pane-base-index 1 with multiple panes.

    This comprehensive test validates the pane ID-based targeting with complex layouts.
    """
    env = isolated_tmux_server
    branch_name = "test-pane-index-fixed"
    window_name = get_window_name(branch_name)

    # Configure tmux with pane-base-index 1
    env.tmux(["set-option", "-g", "pane-base-index", "1"])
    env.tmux(["set-option", "-g", "base-index", "1"])

    # Configure workmux with multiple panes
    write_workmux_config(
        repo_path,
        panes=[
            {"focus": True},
            {"split": "horizontal", "size": 30},
            {"split": "vertical"},
        ],
    )

    # This should succeed after the fix
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Verify the window was created
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows

    # Verify all panes were created (should have 3 panes)
    pane_count = env.tmux(
        ["list-panes", "-t", window_name, "-F", "#{pane_id}"]
    ).stdout.splitlines()
    assert len(pane_count) == 3, f"Expected 3 panes, got {len(pane_count)}"


def test_default_pane_base_index_0_works(
    isolated_tmux_server: TmuxEnvironment, workmux_exe_path: Path, repo_path: Path
):
    """
    Verifies that workmux works correctly with default pane-base-index 0.

    This is a control test to ensure the existing behavior works.
    """
    env = isolated_tmux_server
    branch_name = "test-default-index"
    window_name = get_window_name(branch_name)

    # Explicitly set pane-base-index to 0 (the default)
    env.tmux(["set-option", "-g", "pane-base-index", "0"])

    # Configure workmux with multiple panes
    write_workmux_config(
        repo_path,
        panes=[
            {"focus": True},
            {"split": "horizontal", "size": 30},
            {"split": "vertical"},
        ],
    )

    # This should work fine
    run_workmux_add(env, workmux_exe_path, repo_path, branch_name)

    # Verify the window was created
    list_windows = env.tmux(
        ["list-windows", "-F", "#{window_name}"]
    ).stdout.splitlines()
    assert window_name in list_windows

    # Verify all panes were created
    pane_count = env.tmux(
        ["list-panes", "-t", window_name, "-F", "#{pane_id}"]
    ).stdout.splitlines()
    assert len(pane_count) == 3, f"Expected 3 panes, got {len(pane_count)}"
