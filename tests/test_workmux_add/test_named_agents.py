"""Tests for known agent auto-detection and prompt injection."""

import shlex
from pathlib import Path

from ..conftest import (
    FakeAgentInstaller,
    MuxEnvironment,
    get_window_name,
    wait_for_file,
    write_workmux_config,
)
from .conftest import add_branch_and_get_worktree


class TestKnownAgentAutoDetection:
    """Tests that literal known agent commands auto-detect for prompt injection."""

    def test_literal_known_agent_gets_prompt_injection(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """A literal 'claude' command should auto-detect and inject the prompt."""
        env = mux_server
        branch_name = "feature-auto-detect-claude"
        window_name = get_window_name(branch_name)
        prompt_text = "auto detected prompt"

        fake_agent_installer.install(
            "claude",
            """#!/bin/sh
set -e
printf '%s' "$2" > claude_received.txt
""",
        )

        # Use literal "claude" in panes -- no <agent:> placeholder, no global agent
        write_workmux_config(
            mux_repo_path,
            panes=[{"command": "claude"}],
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt {shlex.quote(prompt_text)}",
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

    def test_two_known_agents_each_get_prompt(
        self,
        mux_server: MuxEnvironment,
        workmux_exe_path: Path,
        mux_repo_path: Path,
        fake_agent_installer: FakeAgentInstaller,
    ):
        """Two literal agent commands should each receive the prompt with their own profile."""
        env = mux_server
        branch_name = "feature-two-auto-detected"
        window_name = get_window_name(branch_name)
        prompt_text = "implement the feature"

        # Claude profile: receives -- "$prompt"
        fake_agent_installer.install(
            "claude",
            """#!/bin/sh
set -e
printf '%s' "$2" > claude_received.txt
""",
        )

        # Gemini profile: receives -i "$prompt"
        fake_agent_installer.install(
            "gemini",
            """#!/bin/sh
set -e
if [ "$1" != "-i" ]; then
    echo "Expected -i flag, got $1" > gemini_error.txt
    exit 1
fi
printf '%s' "$2" > gemini_received.txt
""",
        )

        write_workmux_config(
            mux_repo_path,
            panes=[
                {"command": "claude"},
                {"command": "gemini", "split": "vertical"},
            ],
        )

        worktree_path = add_branch_and_get_worktree(
            env,
            workmux_exe_path,
            mux_repo_path,
            branch_name,
            extra_args=f"--prompt {shlex.quote(prompt_text)}",
        )

        claude_output = worktree_path / "claude_received.txt"
        gemini_output = worktree_path / "gemini_received.txt"

        wait_for_file(
            env,
            claude_output,
            timeout=5.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )
        wait_for_file(
            env,
            gemini_output,
            timeout=5.0,
            window_name=window_name,
            worktree_path=worktree_path,
        )

        assert claude_output.read_text() == prompt_text
        assert gemini_output.read_text() == prompt_text
