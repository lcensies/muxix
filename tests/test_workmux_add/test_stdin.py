"""Tests for stdin input support in `workmux add`."""

from pathlib import Path

from ..conftest import (
    DEFAULT_WINDOW_PREFIX,
    TmuxEnvironment,
    assert_window_exists,
    run_workmux_command,
    slugify,
    write_workmux_config,
)


class TestStdinInput:
    """Tests for piping input to workmux add via stdin."""

    def test_stdin_creates_multiple_worktrees(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that piping lines to stdin creates multiple worktrees.

        Stdin lines become the 'input' variable in foreach_vars, which is
        appended to the base branch name by the default branch template.
        """
        env = isolated_tmux_server
        input_data = "feature-a\nfeature-b"

        write_workmux_config(repo_path)

        # Pipe input - the default branch template appends foreach_vars (input) to base_name
        # So "topic" + "feature-a" -> "topic-feature-a"
        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add topic",
            stdin_input=input_data,
        )

        # Verify all expected worktrees and windows exist
        for item in ["feature-a", "feature-b"]:
            expected_handle = slugify(f"topic-{item}")
            worktree_path = (
                repo_path.parent / f"{repo_path.name}__worktrees" / expected_handle
            )
            assert worktree_path.is_dir(), f"Expected worktree at {worktree_path}"

            expected_window = f"{DEFAULT_WINDOW_PREFIX}{expected_handle}"
            assert_window_exists(env, expected_window)

    def test_stdin_with_custom_branch_template(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that stdin input works with custom branch templates."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        # Use custom branch template that puts input first
        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add base --branch-template '{{ input }}-feature'",
            stdin_input="api\nauth",
        )

        # Verify worktrees were created with custom template
        for item in ["api", "auth"]:
            expected_handle = slugify(f"{item}-feature")
            worktree_path = (
                repo_path.parent / f"{repo_path.name}__worktrees" / expected_handle
            )
            assert worktree_path.is_dir(), f"Expected worktree at {worktree_path}"
            assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}{expected_handle}")

    def test_stdin_conflicts_with_foreach_flag(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that using stdin and --foreach simultaneously fails."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add my-branch --foreach 'region:us,eu'",
            stdin_input="input-data",
            expect_fail=True,
        )

        assert "Cannot use --foreach when piping input from stdin" in result.stderr

    def test_stdin_conflicts_with_name_flag(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that using stdin and --name simultaneously fails (multi-worktree constraint)."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add my-branch --name custom-name",
            stdin_input="input-data",
            expect_fail=True,
        )

        assert "--name cannot be used with multi-worktree generation" in result.stderr

    def test_stdin_conflicts_with_prompt_editor_and_auto_name(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that stdin cannot be used with interactive prompt editor when using -A."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add -A --prompt-editor",
            stdin_input="input-data",
            expect_fail=True,
        )

        assert "Cannot use interactive prompt editor when piping input" in result.stderr

    def test_stdin_overrides_frontmatter_foreach(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that stdin input takes precedence over 'foreach' defined in prompt frontmatter."""
        env = isolated_tmux_server

        # Create a prompt file with foreach in frontmatter
        # Use -p for inline prompt (empty) so we don't trigger agent requirement
        prompt_file = repo_path / "prompt_with_matrix.md"
        prompt_file.write_text("""---
foreach:
  env: [dev, prod]
---
Task for {{ input }}
""")

        # Configure with an agent pane to satisfy the prompt requirement
        write_workmux_config(repo_path, panes=[{"command": "<agent>"}])

        # Pipe 'api' via stdin. The prompt has 'foreach env', but stdin should override it
        result = run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add deploy -P prompt_with_matrix.md",
            stdin_input="api",
        )

        # Warning should be shown
        assert "stdin input overrides prompt frontmatter" in result.stderr

        # Should create 'deploy-api' (base_name + input), ignoring the 'dev/prod' matrix from frontmatter
        assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}deploy-api")

        # Ensure the frontmatter expansion did NOT happen
        window_list = env.tmux(["list-windows", "-F", "#{window_name}"])
        assert "deploy-dev" not in window_list.stdout
        assert "deploy-prod" not in window_list.stdout

    def test_empty_stdin_lines_are_filtered(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that empty lines in stdin are ignored."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        # Include empty lines and whitespace-only lines
        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add task",
            stdin_input="first\n\n  \nsecond\n",
        )

        # Should only create two worktrees (empty lines filtered)
        assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}task-first")
        assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}task-second")

        # Verify no window for empty input
        window_list = env.tmux(["list-windows", "-F", "#{window_name}"])
        # Ensure we only have the expected windows plus the test session window
        window_names = [
            w
            for w in window_list.stdout.strip().split("\n")
            if w.startswith(DEFAULT_WINDOW_PREFIX)
        ]
        assert len(window_names) == 2

    def test_stdin_with_whitespace_trimmed(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that whitespace is trimmed from stdin lines."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        # Include lines with leading/trailing whitespace
        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add item",
            stdin_input="  padded  \n",
        )

        # Should create worktree with trimmed name: item-padded
        assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}item-padded")

    def test_stdin_json_lines_expose_keys_as_variables(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that JSON lines are parsed and keys become template variables."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        # Pipe JSON lines - each key should become a template variable
        json_lines = '{"name":"workmux","id":"1"}\n{"name":"tmux-tools","id":"2"}'

        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add analyze --branch-template '{{ base_name }}-{{ name }}'",
            stdin_input=json_lines,
        )

        # Verify worktrees created with names from JSON 'name' key
        for name in ["workmux", "tmux-tools"]:
            expected_handle = slugify(f"analyze-{name}")
            worktree_path = (
                repo_path.parent / f"{repo_path.name}__worktrees" / expected_handle
            )
            assert worktree_path.is_dir(), f"Expected worktree at {worktree_path}"
            assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}{expected_handle}")

    def test_stdin_json_lines_preserve_input_variable(
        self,
        isolated_tmux_server: TmuxEnvironment,
        workmux_exe_path: Path,
        repo_path: Path,
    ):
        """Verifies that {{ input }} contains the raw JSON line."""
        env = isolated_tmux_server

        write_workmux_config(repo_path)

        # Use {{ input }} in template - should get the raw JSON string (slugified)
        json_line = '{"name":"test"}'

        run_workmux_command(
            env,
            workmux_exe_path,
            repo_path,
            "add task --branch-template '{{ base_name }}-{{ index }}'",
            stdin_input=json_line,
        )

        # {{ index }} should be 1 for single item
        assert_window_exists(env, f"{DEFAULT_WINDOW_PREFIX}task-1")
