"""Tests for named layout selection with -l/--layout."""

import shlex
from pathlib import Path

from ..conftest import (
    FakeAgentInstaller,
    MuxEnvironment,
    get_window_name,
    run_workmux_command,
    wait_for_file,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestLayoutSelection:
    """Tests for selecting a named layout with -l/--layout."""

    def test_layout_selects_panes_from_named_layout(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """The -l flag should use the layout's panes instead of top-level panes."""
        env = mux_server
        branch_name = "feature-layout-basic"
        window_name = get_window_name(branch_name)

        fake_agent_installer.install(
            "claude",
            """#!/bin/sh
echo "layout-agent-ran" > layout_marker.txt
""",
        )

        write_workmux_config(
            mux_repo_path,
            panes=[{"command": "echo top-level"}],
            layouts={
                "design": {
                    "panes": [
                        {"command": "claude"},
                    ]
                }
            },
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args="-l design",
        )

        marker = worktree_path / "layout_marker.txt"
        wait_for_file(
            env,
            marker,
            timeout=5.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert marker.read_text().strip() == "layout-agent-ran"

    def test_layout_with_prompt_injection(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Layout panes with a known agent command should receive prompt injection."""
        env = mux_server
        branch_name = "feature-layout-prompt"
        window_name = get_window_name(branch_name)
        prompt_text = "layout prompt injection test"

        fake_agent_installer.install(
            "claude",
            """#!/bin/sh
printf '%s' "$2" > claude_received.txt
""",
        )

        write_workmux_config(
            mux_repo_path,
            layouts={
                "review": {
                    "panes": [
                        {"command": "claude"},
                    ]
                }
            },
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"-l review --prompt {shlex.quote(prompt_text)}",
        )

        agent_output = worktree_path / "claude_received.txt"
        wait_for_file(
            env,
            agent_output,
            timeout=5.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )
        assert agent_output.read_text() == prompt_text


class TestLayoutErrors:
    """Tests for layout error handling."""

    def test_layout_not_found(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Using a layout name that doesn't exist should fail with available layouts listed."""
        write_workmux_config(
            mux_repo_path,
            layouts={
                "alpha": {"panes": [{"command": "echo a"}]},
                "beta": {"panes": [{"command": "echo b"}]},
            },
        )

        result = run_workmux_command(
            mux_server,
            workmux_exe_path,
            mux_repo_path,
            "add feature-missing-layout -l nonexistent",
            expect_fail=True,
        )
        assert "not found" in result.stderr.lower()
        assert "alpha" in result.stderr
        assert "beta" in result.stderr

    def test_layout_no_layouts_defined(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
    ):
        """Using -l when no layouts are defined should fail with a clear error."""
        write_workmux_config(
            mux_repo_path,
            panes=[{"command": "echo hello"}],
        )

        result = run_workmux_command(
            mux_server,
            workmux_exe_path,
            mux_repo_path,
            "add feature-no-layouts -l somename",
            expect_fail=True,
        )
        assert "no layouts are defined" in result.stderr.lower()
