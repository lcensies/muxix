"""Tests for `workmux add --fork` conversation forking."""

import time
from pathlib import Path


from .conftest import (
    MuxEnvironment,
    get_worktree_path,
    run_workmux_command,
    write_workmux_config,
)


def create_fake_claude_session(
    claude_config_dir: Path,
    worktree_path: Path,
    session_id: str = "test-session-abc123",
    content: str = '{"type":"message"}',
    with_subdir: bool = False,
) -> Path:
    """Create a fake Claude conversation file for testing.

    Returns the path to the created .jsonl file.
    """
    # Encode path the same way Claude does: non-alphanumeric (except -) become -
    encoded = "".join(c if c.isalnum() or c == "-" else "-" for c in str(worktree_path))
    project_dir = claude_config_dir / "projects" / encoded
    project_dir.mkdir(parents=True, exist_ok=True)

    jsonl_path = project_dir / f"{session_id}.jsonl"
    jsonl_path.write_text(content)

    if with_subdir:
        subdir = project_dir / session_id
        subdir.mkdir(exist_ok=True)
        (subdir / "data.json").write_text("{}")

    return jsonl_path


def run_fork_command(
    env: MuxEnvironment,
    workmux_exe_path: Path,
    repo_path: Path,
    command: str,
    claude_dir: Path,
    expect_fail: bool = False,
):
    """Run a workmux add command with CLAUDE_CONFIG_DIR set."""
    return run_workmux_command(
        env,
        workmux_exe_path,
        repo_path,
        f"add {command}",
        expect_fail=expect_fail,
        pre_run_env={"CLAUDE_CONFIG_DIR": str(claude_dir)},
    )


class TestForkBasic:
    """Tests for --fork flag with workmux add."""

    def test_fork_no_conversations_errors(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path, tmp_path
    ):
        """--fork with no conversations in current worktree should fail."""
        env = mux_server
        write_workmux_config(mux_repo_path)

        claude_dir = tmp_path / "claude-empty"
        claude_dir.mkdir()

        result = run_fork_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "fork-test --fork",
            claude_dir,
            expect_fail=True,
        )
        assert "No conversations found" in result.stderr

    def test_fork_copies_conversation(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path, tmp_path
    ):
        """--fork should copy conversation files into the new worktree's project dir."""
        env = mux_server
        write_workmux_config(mux_repo_path)

        claude_dir = tmp_path / "claude"
        session_id = "session-fork-test"

        create_fake_claude_session(
            claude_dir, mux_repo_path, session_id=session_id, with_subdir=True
        )

        run_fork_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "fork-branch --fork",
            claude_dir,
        )

        # Verify worktree was created
        worktree_path = get_worktree_path(mux_repo_path, "fork-branch")
        assert worktree_path.is_dir()

        # Verify conversation was copied to the new worktree's project dir
        encoded_target = "".join(
            c if c.isalnum() or c == "-" else "-" for c in str(worktree_path)
        )
        target_project_dir = claude_dir / "projects" / encoded_target
        assert (target_project_dir / f"{session_id}.jsonl").exists()
        assert (target_project_dir / session_id / "data.json").exists()

    def test_fork_specific_session(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path, tmp_path
    ):
        """--fork=<session-id> should fork a specific conversation."""
        env = mux_server
        write_workmux_config(mux_repo_path)

        claude_dir = tmp_path / "claude"

        # Create two sessions with different mtimes
        create_fake_claude_session(claude_dir, mux_repo_path, session_id="old-session")
        time.sleep(0.1)
        create_fake_claude_session(
            claude_dir, mux_repo_path, session_id="specific-session"
        )

        run_fork_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "fork-specific --fork=specific-session",
            claude_dir,
        )

        worktree_path = get_worktree_path(mux_repo_path, "fork-specific")
        encoded_target = "".join(
            c if c.isalnum() or c == "-" else "-" for c in str(worktree_path)
        )
        target_project_dir = claude_dir / "projects" / encoded_target

        # Only the specific session should be copied
        assert (target_project_dir / "specific-session.jsonl").exists()
        assert not (target_project_dir / "old-session.jsonl").exists()

    def test_fork_unknown_session_errors(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path, tmp_path
    ):
        """--fork=<nonexistent> should fail with clear error."""
        env = mux_server
        write_workmux_config(mux_repo_path)

        claude_dir = tmp_path / "claude"
        create_fake_claude_session(
            claude_dir, mux_repo_path, session_id="existing-session"
        )

        result = run_fork_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "fork-missing --fork=nonexistent",
            claude_dir,
            expect_fail=True,
        )
        assert "No conversation matching 'nonexistent'" in result.stderr

    def test_fork_prefix_match(
        self, mux_server: MuxEnvironment, workmux_exe_path, mux_repo_path, tmp_path
    ):
        """--fork=<prefix> should match session by prefix."""
        env = mux_server
        write_workmux_config(mux_repo_path)

        claude_dir = tmp_path / "claude"
        create_fake_claude_session(
            claude_dir, mux_repo_path, session_id="abc123-def456-full-uuid"
        )

        run_fork_command(
            env,
            workmux_exe_path,
            mux_repo_path,
            "fork-prefix --fork=abc123",
            claude_dir,
        )

        worktree_path = get_worktree_path(mux_repo_path, "fork-prefix")
        encoded_target = "".join(
            c if c.isalnum() or c == "-" else "-" for c in str(worktree_path)
        )
        target_project_dir = claude_dir / "projects" / encoded_target
        assert (target_project_dir / "abc123-def456-full-uuid.jsonl").exists()
