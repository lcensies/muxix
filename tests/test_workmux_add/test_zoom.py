"""Tests for pane zoom configuration in `workmux add`."""

from pathlib import Path

import pytest

from ..conftest import (
    MuxEnvironment,
    TmuxEnvironment,
    get_window_name,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


@pytest.mark.tmux_only
class TestPaneZoom:
    """Tests for zoom: true pane configuration."""

    def test_zoom_pane_is_zoomed(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies that a pane with zoom: true is zoomed after creation."""
        assert isinstance(mux_server, TmuxEnvironment)
        env = mux_server
        branch_name = "feature-zoom"
        window_name = get_window_name(branch_name)

        write_workmux_config(
            mux_repo_path,
            panes=[
                {"command": "echo zoomed", "zoom": True},
                {"command": "echo background", "split": "horizontal", "size": 15},
            ],
        )

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        # Check that the window has the zoomed flag set
        result = env.mux_command(
            ["list-panes", "-t", window_name, "-F", "#{window_zoomed_flag}"]
        )
        flags = result.stdout.strip().split("\n")
        # All panes in a zoomed window report the same zoomed flag
        assert any(f == "1" for f in flags), (
            f"Expected window to be zoomed but got flags: {flags}"
        )

    def test_zoom_implies_focus(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Verifies that zoom: true implies focus on that pane."""
        assert isinstance(mux_server, TmuxEnvironment)
        env = mux_server
        branch_name = "feature-zoom-focus"
        window_name = get_window_name(branch_name)

        write_workmux_config(
            mux_repo_path,
            panes=[
                {"command": "echo first"},
                {
                    "command": "echo zoomed",
                    "split": "horizontal",
                    "size": 15,
                    "zoom": True,
                },
            ],
        )

        add_branch_and_get_worktree(env, workmux_exe_path, mux_repo_path, branch_name)

        # The active pane should be the zoomed one (second pane, index 1)
        result = env.mux_command(
            ["list-panes", "-t", window_name, "-F", "#{pane_active} #{pane_index}"]
        )
        lines = result.stdout.strip().split("\n")
        active_panes = [line for line in lines if line.startswith("1 ")]
        assert len(active_panes) == 1
        # The second pane (index 1) should be active
        assert active_panes[0] == "1 1"
